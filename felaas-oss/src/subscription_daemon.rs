use std::time::Duration;

use anyhow::{bail, Context, Result};
use tokio::time;
use tracing::{debug, error, info};

use crate::common::{ChatUserId, SubscriptionStatus};
use crate::launch::subscription::{
    Subscription, SubscriptionCancellationReason, SubscriptionPayment, SubscriptionPaymentError,
    SubscriptionPaymentId, SubscriptionPaymentStatus,
};
use crate::wallet::core::{FelaasWallet, FelaasWalletError};
use crate::PgPool;

/// Main daemon loop that processes subscriptions and handles payments
/*
- Search for pending subscriptions
- If there is a pending subscription, search for pending subscription payments
- If there are no pending subscription payments, create an invoice on destination wallet, then create a new pending subscription payment with that invoice
- If there are pending subscription payments, get the latest one
- Try to pay the invoice
- If successful, update the subscription status to active and update the payment subscription with preimage
- If unsuccessful, cancel the subscription and cancel the payment subscription with the reason of failure
*/
pub async fn run_standard_daemon(schema: String, pool: PgPool, wallet: &FelaasWallet) {
    // Run the daemon loop continuously
    loop {
        debug!("Try processing pending subscription...");
        match try_process_pending_subscription(&schema, &pool, wallet).await {
            Ok(Some(subscription)) => {
                // Do not sleep if there are pending subscriptions
                debug!(?subscription, "Processed pending subscription");
            }
            Ok(None) => {
                // TODO: create some notification mechanism so we don't need polling/sleep
                time::sleep(Duration::from_secs(1)).await;
            }
            Err(e) => {
                error!(?e, "Error processing pending subscription");
                time::sleep(Duration::from_secs(1)).await;
            }
        }
    }
}

pub async fn try_process_pending_subscription(
    schema: &str,
    pool: &PgPool,
    wallet: &FelaasWallet,
) -> anyhow::Result<Option<(Subscription, SubscriptionPayment)>> {
    let mut client = pool.get().await?;
    let tx = client.transaction().await?;
    // - Search for pending subscriptions
    let pending_subscription =
        crate::launch::subscription::db::get_oldest_pending_subscription(schema, &tx)
            .await
            .context("Failed to find pending subscriptions")?;
    if let Some(pending_subscription) = pending_subscription {
        let subscription_payment =
            crate::launch::subscription::db::find_latest_subscription_payment_by_user(
                schema,
                &tx,
                &pending_subscription.user_id,
            )
            .await
            .context("Failed to find pending subscription payments")?;
        if let Some(subscription_payment) = subscription_payment {
            match &subscription_payment.status {
                SubscriptionPaymentStatus::Pending => {
                    process_subscription_payment(schema, tx, wallet, &subscription_payment)
                        .await
                        .context("Failed to process pending subscription")?;
                    Ok(Some((pending_subscription, subscription_payment)))
                }
                _other => {
                    error!(
                        ?pending_subscription,
                        ?subscription_payment,
                        "Subscription payment is not pending"
                    );
                    bail!("Subscription payment is not pending")
                }
            }
        } else {
            error!(?pending_subscription, "No subscription payment found");
            bail!("No subscription payment found")
        }
    } else {
        Ok(None)
    }
}

/// Process a single subscription payment
async fn process_subscription_payment(
    schema: &str,
    tx: deadpool_postgres::Transaction<'_>,
    wallet: &FelaasWallet,
    payment: &SubscriptionPayment,
) -> Result<()> {
    debug!(?payment, "Processing payment");
    let user_id = &payment.user_id;
    let payment_id = &payment.id;
    let sender_result = wallet
        .pay_invoice(user_id, &payment.payment_request.invoice.clone().into())
        .await;
    match sender_result {
        Ok(sender_result) => {
            // Payment successful on sender side, check if it was successful on receiver
            // side
            let receiver_result = wallet
                .await_internal_invoice(payment.payment_request.operation_id)
                .await;
            match receiver_result {
                Ok(()) => {
                    // Payment successful on both sides, update subscription and payment status
                    let subscription =
                        crate::launch::subscription::db::update_subscription_status(
                            schema,
                            &tx,
                            user_id,
                            SubscriptionStatus::PendingInitialActivation,
                            SubscriptionStatus::Active,
                            None,
                        )
                        .await
                        .context("Failed to update subscription status to active")?;

                    let payment =
                        crate::launch::subscription::db::update_subscription_payment(
                            schema,
                            &tx,
                            payment_id,
                            Some(&sender_result.preimage),
                            None, // No failure reason
                            SubscriptionPaymentStatus::Pending,
                            SubscriptionPaymentStatus::Successful,
                        )
                        .await
                        .context("Failed to update payment status to successful")?;
                    // Commit subscription tx first. In the worst case subscription will be active
                    // with a pending payment that in fact was paid
                    tx.commit().await?;
                    info!(?subscription, ?payment, "Subscription payment successful");
                    Ok(())
                }
                Err(e) => {
                    error!(?e, ?payment, ?sender_result, "Failed to await invoice");
                    // This is a corner case, we can't cancel the subscription payment
                    // because it was paid. let's hope we can retry later and make it work
                    Err(anyhow::anyhow!(
                        "Failed to await invoice: {e} for payment {payment:?}"
                    ))
                }
            }
        }
        Err(FelaasWalletError::InsufficientBalance(details)) => {
            error!(?details, "Insufficient balance to pay for subscription");
            handle_failed_payment(
                schema,
                tx,
                user_id,
                payment_id,
                SubscriptionPaymentError::InsufficientBalance(details),
            )
            .await?;
            bail!("Insufficient balance to pay for subscription")
        }
        Err(e) => {
            let failure_reason = format!("Failed to pay invoice: {e}");
            error!(?e, "Failed to pay invoice");
            handle_failed_payment(
                schema,
                tx,
                user_id,
                payment_id,
                SubscriptionPaymentError::InternalError(failure_reason),
            )
            .await?;
            bail!("Failed to pay invoice: {e}")
        }
    }
}

/// Handle failed payments by cancelling the subscription and payment
async fn handle_failed_payment(
    schema: &str,
    tx: deadpool_postgres::Transaction<'_>,
    user_id: &ChatUserId,
    payment_id: &SubscriptionPaymentId,
    subscription_payment_error: SubscriptionPaymentError,
) -> Result<()> {
    info!(
        ?subscription_payment_error,
        ?user_id,
        "Cancelling subscription and payment"
    );
    // Update subscription status to cancelled
    let subscription = crate::launch::subscription::db::update_subscription_status(
        schema,
        &tx,
        user_id,
        SubscriptionStatus::PendingInitialActivation,
        SubscriptionStatus::Cancelled,
        Some(SubscriptionCancellationReason::SubscriptionPaymentError(
            subscription_payment_error.clone(),
        )),
    )
    .await
    .context("Failed to cancel subscription")?;
    let payment = crate::launch::subscription::db::update_subscription_payment(
        schema,
        &tx,
        payment_id,
        None, // no preimage
        Some(subscription_payment_error),
        SubscriptionPaymentStatus::Pending,
        SubscriptionPaymentStatus::Failed,
    )
    .await
    .context("Failed to cancel payment")?;

    tx.commit().await?;

    info!(
        ?subscription,
        ?payment,
        "Subscription and payment cancelled"
    );

    Ok(())
}

#[cfg(test)]
mod tests {
    use faux::when;
    use fedimint_core::bitcoin::hex::DisplayHex;
    use secrecy::SecretString;

    use super::*;
    use crate::amount::Amount;
    use crate::common::ChatUserId;
    use crate::initialize_logging;
    use crate::launch::subscription::db::SubscriptionDB;
    use crate::launch::subscription::{
        InsufficientBalanceDetails, SubscriptionPaymentError,
    };
    use crate::wallet::core::{InvoiceCreatedDetails, InvoicePaidDetails};

    async fn create_test_pool(pgschema: String) -> Result<PgPool> {
        let pguser = std::env::var(crate::PGUSER)?;
        let pgpassword = std::env::var(crate::PGPASSWORD)
            .ok()
            .map(SecretString::from);
        let pghost = std::env::var(crate::PGHOST)?;
        let pgdatabase = std::env::var(crate::PGDATABASE)?;
        let pgport = std::env::var(crate::PGPORT)?;

        crate::common::create_pg_pool(&crate::PgParams {
            pghost,
            pgport,
            pguser,
            pgpassword,
            pgdatabase,
            pgschema,
        })
        .await
    }

    #[tokio::test]
    async fn test_subscription_happy_path_mocked_wallet() -> Result<()> {
        initialize_logging();

        let schema = format!("schema_{id}", id = uuid::Uuid::new_v4().as_u128());
        let random_user_id = ChatUserId::from(format!("TESTUSER-{id}", id = uuid::Uuid::new_v4()));

        let pool = create_test_pool(schema.clone()).await?;

        let mut wallet = FelaasWallet::faux();
        let plan = crate::launch::subscription::db::get_plan_catalog()
            .first()
            .context("No available plans")?
            .to_owned();

        let random_operation_id = fedimint_client::OperationId::new_random();
        // create invoice
        when!(wallet.create_internal_wallet_invoice(_, _, _, _)).then({
            let plan = plan.clone();
            move |(amount, _, _, _)| {
                assert_eq!(amount, plan.price());
                Ok(InvoiceCreatedDetails {
                    invoice: crate::common::random_invoice(amount)?,
                    operation_id: random_operation_id,
                })
            }
        });

        when!(wallet.pay_invoice(_, _)).then({
            let random_user_id = random_user_id.clone();
            move |(user_id, _)| {
                assert_eq!(*user_id, random_user_id);
                Ok(InvoicePaidDetails {
                    preimage: crate::common::random_preimage().as_hex().to_string(),
                })
            }
        });
        when!(wallet.await_internal_invoice()).then(move |operation_id| {
            assert_eq!(operation_id, random_operation_id);
            Ok(())
        });

        let subscription_db = SubscriptionDB::new(pool.clone(), schema.clone());
        subscription_db.create_table_if_not_exists().await?;
        let allow_internal_invoice = true;
        let subscription = subscription_db
            .subscribe(
                &wallet,
                &random_user_id,
                plan.clone(),
                allow_internal_invoice,
            )
            .await?;
        let processed_subscription = try_process_pending_subscription(&schema, &pool, &wallet)
            .await
            .context("Failed to process pending subscription")?;

        assert_eq!(processed_subscription, Some(subscription));
        Ok(())
    }

    #[tokio::test]
    async fn test_subscription_failed_payment_path_mocked_wallet() -> Result<()> {
        initialize_logging();
        let schema = format!("schema_{id}", id = uuid::Uuid::new_v4().as_u128());
        let random_user_id = ChatUserId::from(format!("TESTUSER-{id}", id = uuid::Uuid::new_v4()));
        let pool = create_test_pool(schema.clone()).await?;
        let mut wallet = FelaasWallet::faux();

        let plan = crate::launch::subscription::db::get_plan_catalog()
            .first()
            .context("No available plans")?
            .to_owned();

        let operation_id = fedimint_client::OperationId::new_random();

        let plan_amount = plan.price();
        // create invoice
        when!(wallet.create_internal_wallet_invoice(_, _, _, _)).then({
            let plan = plan.clone();
            move |(amount, _, _, _)| {
                assert_eq!(amount, plan.price());
                Ok(InvoiceCreatedDetails {
                    invoice: crate::common::random_invoice(amount)?,
                    operation_id,
                })
            }
        });
        when!(wallet.pay_invoice(_, _)).then({
            move |(_, _)| {
                Err(FelaasWalletError::InsufficientBalance(
                    InsufficientBalanceDetails {
                        required: plan_amount,
                        available: Amount::from_msats(0),
                    },
                ))
            }
        });

        let subscription_db = SubscriptionDB::new(pool.clone(), schema.clone());
        subscription_db.create_table_if_not_exists().await?;
        let allow_internal_invoice = true;
        let _ = subscription_db
            .subscribe(
                &wallet,
                &random_user_id,
                plan.clone(),
                allow_internal_invoice,
            )
            .await?;
        let processed_subscription = try_process_pending_subscription(&schema, &pool, &wallet)
            .await
            .context("Failed to process pending subscription");

        info!(?processed_subscription, "Processed subscription");
        assert!(processed_subscription.is_err());

        let subscription = subscription_db
            .get_current_subscription(&random_user_id)
            .await
            .context("Failed to get current subscription")?
            .context("No current subscription")?;
        assert!(subscription.status == SubscriptionStatus::Cancelled);
        let subscription_payment = subscription_db
            .find_latest_subscription_payment_by_user(&random_user_id)
            .await?
            .context("Failed to find latest subscription payment")?;
        assert!(
            subscription_payment
                .failure_reason
                .context("Missing reason")?
                == SubscriptionPaymentError::InsufficientBalance(InsufficientBalanceDetails {
                    required: plan_amount,
                    available: Amount::from_msats(0),
                })
        );

        // TODO: now try to top up the wallet and reactivate the subscription
        Ok(())
    }
}
