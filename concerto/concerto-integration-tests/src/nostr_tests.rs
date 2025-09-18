// Nostr coordination integration tests

use anyhow::Result;
use kube::Client;
use nostr_sdk::prelude::*;
use nostr_sdk::{SingleLetterTag, Alphabet};
use std::borrow::Cow;
use tracing::{info, debug};
use uuid::Uuid;

use crate::common::{self, EnvConf, PgPool};

/// Test basic Nostr relay connectivity
pub async fn test_nostr_relay_connectivity(
    client: &Client,
    env_conf: &EnvConf,
) -> Result<()> {
    info!("Testing Nostr relay connectivity");
    
    // Create test keys
    let keys = Keys::generate();
    info!("Generated test keys: {}", keys.public_key().to_bech32()?);
    
    // Connect to relay
    let nostr_client = nostr_sdk::Client::new(keys.clone());
    nostr_client.add_relay(&env_conf.nostr_relay_url).await?;
    nostr_client.connect().await;
    
    // Wait for connection
    tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
    
    // Publish a test event
    let test_content = format!("Integration test event: {}", Uuid::new_v4());
    let event = EventBuilder::new(Kind::from(1), test_content.clone())
        .build(keys.public_key())
        .sign_with_keys(&keys)?;
    
    let output = nostr_client.send_event(&event).await?;
    let event_id = *output.id();
    info!("Published test event with ID: {}", event_id.to_hex());
    
    // Subscribe and verify we can receive it
    let filter = Filter::new()
        .id(event_id)
        .limit(1);
    
    let events = nostr_client.fetch_events(
        filter,
        tokio::time::Duration::from_secs(5)
    ).await?;
    
    assert!(!events.is_empty(), "Should receive the test event");
    assert_eq!(events.iter().nth(0).unwrap().content, test_content, "Event content should match");
    
    info!("✓ Nostr relay connectivity test passed");
    Ok(())
}

/// Test multi-guardian Nostr messaging
pub async fn test_multi_guardian_messaging(
    client: &Client,
    env_conf: &EnvConf,
    pool: &PgPool,
) -> Result<()> {
    info!("Testing multi-guardian Nostr messaging");
    
    let test_id = Uuid::new_v4();
    let namespace = common::create_test_namespace(client, "nostr-test", test_id).await?;
    
    // Create 3 guardian keys
    let guardian1_keys = Keys::generate();
    let guardian2_keys = Keys::generate();
    let guardian3_keys = Keys::generate();
    
    info!("Guardian 1: {}", guardian1_keys.public_key().to_bech32()?);
    info!("Guardian 2: {}", guardian2_keys.public_key().to_bech32()?);
    info!("Guardian 3: {}", guardian3_keys.public_key().to_bech32()?);
    
    // Create Nostr clients for each guardian
    let client1 = nostr_sdk::Client::new(guardian1_keys.clone());
    let client2 = nostr_sdk::Client::new(guardian2_keys.clone());
    let client3 = nostr_sdk::Client::new(guardian3_keys.clone());
    
    // Connect all to relay
    for client in [&client1, &client2, &client3] {
        client.add_relay(&env_conf.nostr_relay_url).await?;
        client.connect().await;
    }
    
    // Wait for connections
    tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
    
    // Test 1: Broadcast message from guardian 1
    let federation_id = format!("test-fed-{}", test_id.simple());
    let test_message = serde_json::json!({
        "type": "federation_proposal",
        "federation_id": federation_id,
        "guardians": [
            guardian1_keys.public_key().to_bech32()?,
            guardian2_keys.public_key().to_bech32()?,
            guardian3_keys.public_key().to_bech32()?,
        ],
        "slots": 4,
    });
    
    let event = EventBuilder::new(
        Kind::from(30500), // Federation proposal event
        serde_json::to_string(&test_message)?
    )
        .tag(Tag::custom(
            TagKind::Custom(Cow::Borrowed("federation")),
            vec![federation_id.clone()]
        ))
        .build(guardian1_keys.public_key())
        .sign_with_keys(&guardian1_keys)?;
    
    let output = client1.send_event(&event).await?;
    let event_id = *output.id();
    info!("Guardian 1 broadcast federation proposal: {}", event_id.to_hex());
    
    // Test 2: Other guardians should receive it
    let filter = Filter::new()
        .kind(Kind::from(30500))
        .custom_tag(
            SingleLetterTag::lowercase(Alphabet::F),
            federation_id.clone()
        )
        .since(Timestamp::now());
    
    // Guardian 2 receives
    let events2 = client2.fetch_events(
        filter.clone(),
        tokio::time::Duration::from_secs(5)
    ).await?;
    assert!(!events2.is_empty(), "Guardian 2 should receive the proposal");
    
    // Guardian 3 receives
    let events3 = client3.fetch_events(
        filter,
        tokio::time::Duration::from_secs(5)
    ).await?;
    assert!(!events3.is_empty(), "Guardian 3 should receive the proposal");
    
    info!("All guardians received the federation proposal");
    
    // Test 3: Encrypted direct message between guardians
    let dm_content = "Secret setup information";
    let encrypted = nip44::encrypt(
        &guardian1_keys.secret_key(),
        &guardian2_keys.public_key(),
        dm_content.as_bytes(),
        nip44::Version::V2,
    )?;
    
    let dm_event = EventBuilder::new(Kind::from(1059), encrypted.clone()) // NIP-44 encrypted DM
        .tag(Tag::public_key(guardian2_keys.public_key()))
        .build(guardian1_keys.public_key())
        .sign_with_keys(&guardian1_keys)?;
    
    client1.send_event(&dm_event).await?;
    info!("Guardian 1 sent encrypted DM to Guardian 2");
    
    // Guardian 2 decrypts the message
    let dm_filter = Filter::new()
        .kind(Kind::from(1059))
        .pubkey(guardian1_keys.public_key())
        .since(Timestamp::now());
    
    let dm_events = client2.fetch_events(
        dm_filter,
        tokio::time::Duration::from_secs(5)
    ).await?;
    
    assert!(!dm_events.is_empty(), "Guardian 2 should receive the DM");
    
    let decrypted = nip44::decrypt(
        &guardian2_keys.secret_key(),
        &guardian1_keys.public_key(),
        &dm_events.iter().nth(0).unwrap().content,
    )?;
    assert_eq!(decrypted, dm_content.as_bytes(), "Decrypted content should match");
    
    info!("✓ Encrypted DM successfully sent and decrypted");
    
    // Cleanup
    common::cleanup_namespace(client, &namespace).await?;
    
    info!("✓ Multi-guardian messaging test passed");
    Ok(())
}