use std::path::PathBuf;
use std::sync::Arc;

use anyhow::bail;
use fedimint_bip39::Bip39RootSecretStrategy;
use fedimint_client::secret::RootSecretStrategy;
use fedimint_client::{Client, ClientHandleArc, RootSecret};
use fedimint_core::db::Database;
use fedimint_core::invite_code::InviteCode;
use fedimint_core::module::registry::ModuleRegistry;
use fedimint_ln_client::LightningClientInit;
use fedimint_mint_client::MintClientInit;
use fedimint_wallet_client::WalletClientInit;
use postgres_types::{FromSql, ToSql};
use serde::{Deserialize, Serialize};
use tracing::debug;
use url::Url;

use crate::{PgParams, PgPool};

/// Newtype for user IDs to improve type safety across the codebase.
#[derive(
    Clone,
    Debug,
    PartialEq,
    Eq,
    Hash,
    PartialOrd,
    Ord,
    serde::Serialize,
    serde::Deserialize,
    FromSql,
)]
#[serde(transparent)]
pub struct ChatUserId(String);

impl From<String> for ChatUserId {
    fn from(s: String) -> Self {
        ChatUserId(s)
    }
}

impl From<&str> for ChatUserId {
    fn from(s: &str) -> Self {
        ChatUserId(s.to_owned())
    }
}

impl AsRef<str> for ChatUserId {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for ChatUserId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl ToSql for ChatUserId {
    fn to_sql(
        &self,
        ty: &postgres_types::Type,
        out: &mut tokio_util::bytes::BytesMut,
    ) -> Result<postgres_types::IsNull, Box<dyn std::error::Error + Sync + Send>>
    where
        Self: Sized,
    {
        self.0.to_sql(ty, out)
    }

    fn accepts(ty: &postgres_types::Type) -> bool
    where
        Self: Sized,
    {
        <String as ToSql>::accepts(ty)
    }

    fn to_sql_checked(
        &self,
        ty: &postgres_types::Type,
        out: &mut tokio_util::bytes::BytesMut,
    ) -> Result<postgres_types::IsNull, Box<dyn std::error::Error + Sync + Send>> {
        self.0.to_sql_checked(ty, out)
    }
}

async fn load_or_generate_mnemonic(db: &Database) -> anyhow::Result<fedimint_bip39::Mnemonic> {
    let mnemonic = if let Ok(entropy) = Client::load_decodable_client_secret::<Vec<u8>>(db).await {
        fedimint_bip39::Mnemonic::from_entropy(&entropy)?
    } else {
        debug!("Generating mnemonic and writing entropy to client storage");
        let mnemonic = fedimint_bip39::Bip39RootSecretStrategy::<12>::random(
            &mut fedimint_core::secp256k1::rand::thread_rng(),
        );
        Client::store_encodable_client_secret(db, mnemonic.to_entropy()).await?;
        mnemonic
    };
    Ok(mnemonic)
}

/// Convenience function to derive fedimint-client root secret
/// using the default (0) wallet number, given a global root secret
/// that's managed externally by a consumer of fedimint-client.
///
/// See docs/secret_derivation.md
///
/// `global_root_secret/<key-type=per-federation=0>/<federation-id>/
/// <wallet-number=0>/<key-type=fedimint-client=0>`
pub fn get_default_client_secret(
    global_root_secret: &fedimint_derive_secret::DerivableSecret,
    federation_id: &fedimint_core::config::FederationId,
) -> fedimint_derive_secret::DerivableSecret {
    let multi_federation_root_secret =
        global_root_secret.child_key(fedimint_derive_secret::ChildId(0));
    let federation_root_secret = multi_federation_root_secret.federation_key(federation_id);
    let federation_wallet_root_secret =
        federation_root_secret.child_key(fedimint_derive_secret::ChildId(0)); // wallet-number=0
    federation_wallet_root_secret.child_key(fedimint_derive_secret::ChildId(0)) // key-type=fedimint-client=0
}

pub async fn build_client(
    invite_code: Option<&InviteCode>,
    rocksdb: &PathBuf,
) -> anyhow::Result<ClientHandleArc> {
    let db = Database::new(
        fedimint_cursed_redb::MemAndRedb::new(rocksdb).await?,
        ModuleRegistry::default(),
    );
    let mut client_builder = Client::builder(db).await?;
    client_builder.with_module(MintClientInit);
    client_builder.with_module(LightningClientInit::default());
    client_builder.with_module(WalletClientInit::default());
    client_builder.with_primary_module_kind(fedimint_mint_client::KIND);

    if Client::is_initialized(client_builder.db_no_decoders()).await {
        debug!("Client is already initialized");
        let secret =
            Client::load_decodable_client_secret::<Vec<u8>>(client_builder.db_no_decoders())
                .await?;
        let mnemonic = fedimint_bip39::Mnemonic::from_entropy(&secret)?;
        let config = client_builder.load_existing_config().await?;
        let federation_id = config.calculate_federation_id();
        let client = client_builder
            .open(RootSecret::Custom(get_default_client_secret(
                &fedimint_bip39::Bip39RootSecretStrategy::<12>::to_root_secret(&mnemonic),
                &federation_id,
            )))
            .await
            .map(Arc::new)?;
        Ok(client)
    } else if let Some(invite_code) = &invite_code {
        debug!(
            ?invite_code,
            "Client is not initialized, joining with invite code"
        );
        let mnemonic = load_or_generate_mnemonic(client_builder.db_no_decoders()).await?;
        let client_config = client_builder.preview(invite_code).await?;
        let federation_id = client_config.config().global.calculate_federation_id();
        let root_secret = RootSecret::Custom(get_default_client_secret(
            &Bip39RootSecretStrategy::<12>::to_root_secret(&mnemonic),
            &federation_id,
        ));
        let client = client_config.join(root_secret).await.map(Arc::new)?;
        Ok(client)
    } else {
        bail!("Database not initialized and invite code not provided");
    }
}

pub async fn create_pg_pool(pg: &PgParams) -> anyhow::Result<PgPool> {
    // TODO: limit number of connections to a reasonable value
    let mut cfg = deadpool_postgres::Config::new();
    cfg.host = Some(pg.pghost.clone());
    cfg.port = Some(
        pg.pgport
            .parse()
            .map_err(|e| anyhow::anyhow!("Invalid Postgres port: {e}"))?,
    );
    cfg.user = Some(pg.pguser.clone());
    cfg.password = pg.pgpassword.clone();
    cfg.dbname = Some(pg.pgdatabase.clone());
    let pool = cfg.create_pool(
        Some(deadpool_postgres::Runtime::Tokio1),
        tokio_postgres::NoTls,
    )?;
    Ok(pool)
}

#[derive(Debug, Clone, Serialize, Deserialize, Hash, PartialEq, Eq, PartialOrd, Ord)]
pub struct Endpoint(Url);

impl From<Url> for Endpoint {
    fn from(url: Url) -> Self {
        Endpoint(url)
    }
}

impl ToSql for Endpoint {
    fn to_sql(
        &self,
        ty: &postgres_types::Type,
        out: &mut tokio_util::bytes::BytesMut,
    ) -> Result<postgres_types::IsNull, Box<dyn std::error::Error + Sync + Send>> {
        self.0.as_str().to_sql(ty, out)
    }

    fn accepts(ty: &postgres_types::Type) -> bool
    where
        Self: Sized,
    {
        <&str as ToSql>::accepts(ty)
    }

    fn to_sql_checked(
        &self,
        ty: &postgres_types::Type,
        out: &mut tokio_util::bytes::BytesMut,
    ) -> Result<postgres_types::IsNull, Box<dyn std::error::Error + Sync + Send>> {
        self.0.as_str().to_sql_checked(ty, out)
    }
}

impl<'a> FromSql<'a> for Endpoint {
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

impl std::str::FromStr for Endpoint {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self(Url::parse(s)?))
    }
}

impl Endpoint {
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

// Note: this flow doesn't handle renewals yet, we probably should add more
// states
#[derive(
    Debug,
    Copy,
    Clone,
    Hash,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Serialize,
    Deserialize,
    strum::EnumString,
    strum::Display,
)]
pub enum SubscriptionStatus {
    PendingInitialActivation,
    Active,
    Cancelled,
}

impl ToSql for SubscriptionStatus {
    fn to_sql(
        &self,
        ty: &postgres_types::Type,
        out: &mut tokio_util::bytes::BytesMut,
    ) -> std::result::Result<postgres_types::IsNull, Box<dyn std::error::Error + Sync + Send>> {
        self.to_string().to_sql(ty, out)
    }

    fn accepts(ty: &postgres_types::Type) -> bool {
        <&str as ToSql>::accepts(ty)
    }

    fn to_sql_checked(
        &self,
        ty: &postgres_types::Type,
        out: &mut tokio_util::bytes::BytesMut,
    ) -> std::result::Result<postgres_types::IsNull, Box<dyn std::error::Error + Sync + Send>> {
        self.to_string().to_sql_checked(ty, out)
    }
}

impl<'a> FromSql<'a> for SubscriptionStatus {
    fn from_sql(
        _ty: &postgres_types::Type,
        raw: &'a [u8],
    ) -> std::result::Result<Self, Box<dyn std::error::Error + Sync + Send>> {
        let s = std::str::from_utf8(raw)?;
        Ok(std::str::FromStr::from_str(s)?)
    }

    fn accepts(ty: &postgres_types::Type) -> bool {
        <&str as FromSql>::accepts(ty)
    }
}

#[cfg(test)]
pub fn random_preimage() -> [u8; 32] {
    fedimint_core::secp256k1::rand::Rng::r#gen(&mut fedimint_core::secp256k1::rand::thread_rng())
}

#[cfg(test)]
pub fn random_invoice(
    amount: crate::amount::Amount,
) -> anyhow::Result<crate::wallet::core::Invoice> {
    let private_key =
        fedimint_core::secp256k1::SecretKey::new(&mut fedimint_core::secp256k1::rand::thread_rng());

    let preimage = random_preimage();
    let payment_hash =
        <fedimint_core::bitcoin::hashes::sha256::Hash as fedimint_core::BitcoinHash>::hash(
            &preimage,
        );
    let payment_secret: [u8; 32] = fedimint_core::secp256k1::rand::Rng::r#gen(
        &mut fedimint_core::secp256k1::rand::thread_rng(),
    );
    let payment_secret = fedimint_ln_common::lightning_invoice::PaymentSecret(payment_secret);

    let invoice = fedimint_ln_common::lightning_invoice::InvoiceBuilder::new(
        fedimint_ln_common::lightning_invoice::Currency::Bitcoin,
    )
    .description("Coins pls!".into())
    .payment_hash(payment_hash)
    .payment_secret(payment_secret)
    .current_timestamp()
    .min_final_cltv_expiry_delta(144)
    .amount_milli_satoshis(amount.msats)
    .build_signed(|hash| {
        fedimint_core::secp256k1::Secp256k1::new().sign_ecdsa_recoverable(hash, &private_key)
    })?;

    Ok(crate::wallet::core::Invoice::from(invoice))
}

#[cfg(test)]
pub fn mock_random_invoice(
    mut mocked_wallet: crate::wallet::core::FelaasWallet,
) -> crate::wallet::core::FelaasWallet {
    faux::when!(mocked_wallet.create_internal_wallet_invoice(_, _, _, _)).then({
        move |(amount, _, _, _)| {
            Ok(crate::wallet::core::InvoiceCreatedDetails {
                invoice: crate::common::random_invoice(amount)?,
                operation_id: fedimint_client::OperationId::new_random(),
            })
        }
    });

    faux::when!(mocked_wallet.pay_invoice(_, _)).then({
        move |(_, _)| {
            Ok(crate::wallet::core::InvoicePaidDetails {
                preimage: fedimint_core::bitcoin::hex::DisplayHex::as_hex(
                    &crate::common::random_preimage(),
                )
                .to_string(),
            })
        }
    });

    // mock await invoice
    faux::when!(mocked_wallet.await_internal_invoice()).then(move |_operation_id| Ok(()));

    mocked_wallet
}
