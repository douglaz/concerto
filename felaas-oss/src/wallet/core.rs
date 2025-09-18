use std::num::NonZero;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Result, anyhow};
use fedimint_client::{ClientHandleArc, OperationId};
use fedimint_core::invite_code::InviteCode;
use fedimint_ln_client::{InternalPayState, LnPayState, LnReceiveState, PayType};
use fedimint_ln_common::lightning_invoice::{self, Bolt11Invoice, Description};
use futures::StreamExt;
use lru::LruCache;
use postgres_types::{FromSql, ToSql};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::sync::RwLock;
use tracing::{debug, error, warn};

use crate::amount::Amount;
use crate::common::{self, ChatUserId};
use crate::launch::subscription::InsufficientBalanceDetails;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Invoice(Bolt11Invoice);

impl std::str::FromStr for Invoice {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self(Bolt11Invoice::from_str(s)?))
    }
}

impl std::fmt::Display for Invoice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<Invoice> for Bolt11Invoice {
    fn from(invoice: Invoice) -> Self {
        invoice.0
    }
}

impl From<Bolt11Invoice> for Invoice {
    fn from(invoice: Bolt11Invoice) -> Self {
        Self(invoice)
    }
}

impl ToSql for Invoice {
    fn to_sql(
        &self,
        ty: &postgres_types::Type,
        out: &mut tokio_util::bytes::BytesMut,
    ) -> Result<postgres_types::IsNull, Box<dyn std::error::Error + Sync + Send>> {
        self.0.to_string().to_sql(ty, out)
    }

    fn accepts(ty: &postgres_types::Type) -> bool {
        <&str as ToSql>::accepts(ty)
    }

    fn to_sql_checked(
        &self,
        ty: &postgres_types::Type,
        out: &mut tokio_util::bytes::BytesMut,
    ) -> Result<postgres_types::IsNull, Box<dyn std::error::Error + Sync + Send>> {
        self.0.to_string().to_sql_checked(ty, out)
    }
}

impl<'a> FromSql<'a> for Invoice {
    fn from_sql(
        _ty: &postgres_types::Type,
        raw: &'a [u8],
    ) -> Result<Self, Box<dyn std::error::Error + Sync + Send>> {
        let s = std::str::from_utf8(raw)?;
        Ok(std::str::FromStr::from_str(s)?)
    }

    fn accepts(ty: &postgres_types::Type) -> bool {
        <&str as FromSql>::accepts(ty)
    }
}

#[derive(Serialize, Debug)]
pub struct InvoicePaidDetails {
    pub preimage: String,
}

#[derive(Serialize, Debug)]
pub struct InvoiceCreatedDetails {
    pub operation_id: OperationId,
    pub invoice: Invoice,
}

#[derive(Serialize, Debug)]
pub struct BalanceDetails {
    pub amount_msats: Amount,
}

/// Business validation and error states for FelaasWallet operations.
#[derive(Debug, Error)]
pub enum FelaasWalletError {
    #[error("Invoice amount lower or equal to zero")]
    InvoiceAmountEqualZero,

    #[error("No invoice amount specified")]
    NoInvoiceAmount,

    #[error("Invoice canceled")]
    InvoiceCanceled,

    #[error("Unknown operation id")]
    UnknownOperationId,

    #[error("Insufficient balance")]
    InsufficientBalance(InsufficientBalanceDetails),

    #[error("Other error: {0}")]
    Other(#[from] anyhow::Error),
}

#[cfg_attr(test, faux::create)]
#[derive(Clone)]
pub struct FelaasWallet {
    invite_code: InviteCode,
    db_path: PathBuf,
    internal_user_wallet: ClientHandleArc,
    // LruCache is not thread-safe, so concurrency must be handled externally
    clients_by_user_id: Arc<RwLock<LruCache<ChatUserId, ClientHandleArc>>>,
}

#[cfg_attr(test, faux::methods)]
impl FelaasWallet {
    /// Constructs a new FelaasWallet instance.
    pub async fn new(
        invite_code: InviteCode,
        db_path: PathBuf,
        internal_user_id: ChatUserId,
        capacity: NonZero<usize>,
    ) -> anyhow::Result<Self> {
        let internal_user_wallet =
            build_for_user(&invite_code, &db_path, &internal_user_id).await?;
        Ok(Self {
            invite_code,
            db_path,
            internal_user_wallet,
            clients_by_user_id: Arc::new(RwLock::new(LruCache::new(capacity))),
        })
    }

    // get or join the user to the main federation
    pub async fn get_or_join(&self, user_id: &ChatUserId) -> anyhow::Result<ClientHandleArc> {
        if let Some(client) = self.clients_by_user_id.write().await.get(user_id) {
            return Ok(std::sync::Arc::clone(client));
        }
        let client = build_for_user(&self.invite_code, &self.db_path, user_id).await?;
        self.clients_by_user_id
            .write()
            .await
            .push(user_id.to_owned(), client.clone());
        Ok(client)
    }

    pub async fn create_internal_wallet_invoice(
        &self,
        amount: Amount,
        expire_time_secs: Option<u64>,
        desc: Description,
        allow_internal_invoice: bool,
    ) -> Result<InvoiceCreatedDetails, FelaasWalletError> {
        let internal_user_wallet = self.internal_user_wallet.clone();
        create_invoice(
            &internal_user_wallet,
            amount,
            expire_time_secs,
            desc,
            allow_internal_invoice,
        )
        .await
    }

    pub async fn create_invoice(
        &self,
        user_id: &ChatUserId,
        amount: Amount,
        expire_time_secs: Option<u64>,
        desc: Description,
        allow_internal: bool,
    ) -> Result<InvoiceCreatedDetails, FelaasWalletError> {
        let client = self.get_or_join(user_id).await?;
        create_invoice(&client, amount, expire_time_secs, desc, allow_internal).await
    }

    pub async fn pay_invoice(
        &self,
        user_id: &ChatUserId,
        invoice: &Bolt11Invoice,
    ) -> Result<InvoicePaidDetails, FelaasWalletError> {
        let client = self.get_or_join(user_id).await?;
        let lightning = client.get_first_module::<fedimint_ln_client::LightningClientModule>()?;
        match invoice.amount_milli_satoshis() {
            Some(0) => {
                return Err(FelaasWalletError::InvoiceAmountEqualZero);
            }
            Some(amount_milli_satoshis) => {
                let invoice_amount = Amount::from_msats(amount_milli_satoshis);
                let balance = Amount::from(client.get_balance().await);
                if balance < invoice_amount {
                    return Err(FelaasWalletError::InsufficientBalance(
                        InsufficientBalanceDetails {
                            required: invoice_amount,
                            available: balance,
                        },
                    ));
                }
            }
            None => {
                return Err(FelaasWalletError::NoInvoiceAmount);
            }
        }

        let result = lightning
            .pay_bolt11_invoice(None, invoice.clone(), ())
            .await?;

        match result.payment_type {
            PayType::Internal(operation_id) => {
                let mut updates = lightning
                    .subscribe_internal_pay(operation_id)
                    .await?
                    .into_stream();

                while let Some(update) = updates.next().await {
                    match update {
                        InternalPayState::Preimage(preimage) => {
                            return Ok(InvoicePaidDetails {
                                preimage: preimage.to_string(),
                            });
                        }
                        InternalPayState::RefundSuccess { out_points, error } => {
                            let e = format!(
                                "Internal payment failed. A refund was issued to {out_points:?} Error: {error}"
                            );
                            return Err(anyhow!(e).into());
                        }
                        InternalPayState::UnexpectedError(e) => {
                            return Err(anyhow!("Unexpected error: {e}").into());
                        }
                        InternalPayState::Funding => {
                            debug!(%invoice, ?user_id, "InternalPayState::Funding");
                        }
                        InternalPayState::RefundError {
                            error_message,
                            error,
                        } => return Err(anyhow!("RefundError: {error_message} {error}").into()),
                        InternalPayState::FundingFailed { error } => {
                            return Err(anyhow!("FundingFailed: {error}").into());
                        }
                    }
                    debug!(%invoice, %user_id, ?update, "Wait for ln payment state update");
                }
            }
            PayType::Lightning(operation_id) => {
                let mut updates = lightning
                    .subscribe_ln_pay(operation_id)
                    .await?
                    .into_stream();

                while let Some(update) = updates.next().await {
                    match update {
                        LnPayState::Success { preimage } => {
                            return Ok(InvoicePaidDetails { preimage });
                        }
                        LnPayState::Refunded { gateway_error } => {
                            // TODO: what should be the format here?
                            return Err(anyhow!("Refunded: {gateway_error}").into());
                        }

                        LnPayState::Created
                        | LnPayState::AwaitingChange
                        | LnPayState::WaitingForRefund { .. }
                        | LnPayState::Funded { block_height: _ } => {}
                        LnPayState::UnexpectedError { error_message } => {
                            return Err(anyhow!("UnexpectedError: {error_message}").into());
                        }
                        LnPayState::Canceled => {
                            return Err(anyhow!("Funding transaction was rejected").into());
                        }
                    }
                    debug!(%invoice, %user_id, ?update, "Wait for ln payment state update");
                }
            }
        };
        Err(anyhow!("Lightning Payment failed").into())
    }

    pub async fn await_internal_invoice(
        &self,
        operation_id: fedimint_client::OperationId,
    ) -> Result<(), FelaasWalletError> {
        let internal_user_wallet = self.internal_user_wallet.clone();
        await_invoice(&internal_user_wallet, operation_id).await
    }

    pub async fn await_invoice(
        &self,
        user_id: &ChatUserId,
        operation_id: fedimint_client::OperationId,
    ) -> Result<(), FelaasWalletError> {
        let client = self.get_or_join(user_id).await?;
        await_invoice(&client, operation_id).await
    }

    pub async fn get_balance(
        &self,
        user_id: &ChatUserId,
    ) -> Result<BalanceDetails, FelaasWalletError> {
        let client = self.get_or_join(user_id).await?;
        Ok(BalanceDetails {
            amount_msats: client.get_balance().await.into(),
        })
    }
}

pub async fn create_invoice(
    client: &ClientHandleArc,
    amount: Amount,
    expire_time_secs: Option<u64>,
    desc: Description,
    allow_internal_invoice: bool,
) -> Result<InvoiceCreatedDetails, FelaasWalletError> {
    let lightning = client.get_first_module::<fedimint_ln_client::LightningClientModule>()?;
    lightning.update_gateway_cache().await?;
    let gateways = lightning.list_gateways().await;

    let gateway = gateways.into_iter().max_by_key(|g| g.ttl);

    if gateway.is_none() && !allow_internal_invoice {
        return Err(anyhow::anyhow!("No gateway found").into());
    }

    let (operation_id, invoice, _) = lightning
        .create_bolt11_invoice(
            amount.into(),
            lightning_invoice::Bolt11InvoiceDescription::Direct(desc),
            expire_time_secs,
            (),
            gateway.map(|g| g.info),
        )
        .await?;
    Ok(InvoiceCreatedDetails {
        operation_id,
        invoice: Invoice(invoice),
    })
}

pub async fn await_invoice(
    client: &ClientHandleArc,
    operation_id: fedimint_client::OperationId,
) -> Result<(), FelaasWalletError> {
    if !client.operation_exists(operation_id).await {
        warn!(
            operation_id = %operation_id.fmt_short(),
            "Operation id not found for this user"
        );
        return Err(FelaasWalletError::UnknownOperationId);
    }

    let lightning_module =
        &client.get_first_module::<fedimint_ln_client::LightningClientModule>()?;

    let mut updates = match lightning_module.subscribe_ln_receive(operation_id).await {
        Ok(subscription) => subscription.into_stream(),
        Err(e) => {
            error!(
                ?e,
                ?operation_id,
                "Failed to subscribe ln receive with this operation id"
            );
            return Err(anyhow!("Failed to subscribe ln receive with this operation id").into());
        }
    };

    while let Some(update) = updates.next().await {
        match update {
            LnReceiveState::Claimed => return Ok(()),
            LnReceiveState::Canceled { reason } => {
                warn!("Invoice canceled: {reason:?}");
                return Err(FelaasWalletError::InvoiceCanceled);
            }
            other => {
                debug!(?other, "Ignored event type update");
            }
        }
    }

    Err(anyhow!(
        "No updates for invoice, operation_id: {operation_id}",
        operation_id = operation_id.fmt_short()
    )
    .into())
}

pub async fn build_for_user(
    invite_code: &InviteCode,
    db_path: &Path,
    user_id: &ChatUserId,
) -> anyhow::Result<ClientHandleArc> {
    let rocksdb = db_path.join(user_id.as_ref()).join("client.db");
    let client = common::build_client(Some(invite_code), &rocksdb).await?;
    Ok(client)
}
