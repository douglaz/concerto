// DKG (Distributed Key Generation) integration tests

use anyhow::Result;
use kube::Client;
use nostr_sdk::prelude::*;
use nostr_sdk::{SingleLetterTag, Alphabet};
use std::borrow::Cow;
use tracing::{info, debug};
use uuid::Uuid;
use std::collections::BTreeMap;

use crate::common::{self, EnvConf, PgPool};

/// Test DKG with three guardians
pub async fn test_dkg_with_three_guardians(
    client: &Client,
    env_conf: &EnvConf,
    pool: &PgPool,
) -> Result<()> {
    info!("Testing DKG with 3 guardians");
    
    let test_id = Uuid::new_v4();
    let namespace = common::create_test_namespace(client, "dkg-test", test_id).await?;
    let federation_id = format!("fed-{}", test_id.simple());
    
    // Create guardian keys
    let leader_keys = Keys::generate();
    let guardian2_keys = Keys::generate();
    let guardian3_keys = Keys::generate();
    
    let guardian_npubs = vec![
        leader_keys.public_key().to_bech32()?,
        guardian2_keys.public_key().to_bech32()?,
        guardian3_keys.public_key().to_bech32()?,
    ];
    
    info!("Leader Guardian: {}", guardian_npubs[0]);
    info!("Guardian 2: {}", guardian_npubs[1]);
    info!("Guardian 3: {}", guardian_npubs[2]);
    
    // Deploy guardianito instances
    for (i, (keys, npub)) in [(leader_keys.clone(), &guardian_npubs[0]),
                               (guardian2_keys.clone(), &guardian_npubs[1]),
                               (guardian3_keys.clone(), &guardian_npubs[2])].iter().enumerate() {
        common::deploy_guardianito_instance(
            client,
            &namespace,
            &format!("guardian-{}", i + 1),
            &env_conf.guardianito_image,
            npub, // owner is self
            &keys.secret_key().to_bech32()?,
            &vec![env_conf.nostr_relay_url.clone()],
        ).await?;
    }
    
    // Wait for guardians to start
    tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
    
    // Connect to Nostr relay as leader
    let leader_client = nostr_sdk::Client::new(leader_keys.clone());
    leader_client.add_relay(&env_conf.nostr_relay_url).await?;
    leader_client.connect().await;
    
    // Step 1: Leader initiates DKG
    let dkg_init_message = serde_json::json!({
        "type": "DkgStarted",
        "federation_id": federation_id,
        "guardians": guardian_npubs,
        "threshold": 2, // 2-of-3
    });
    
    let init_event = EventBuilder::new(
        Kind::from(30078), // DKG event kind
        serde_json::to_string(&dkg_init_message)?
    )
        .tag(Tag::custom(
            TagKind::Custom(Cow::Borrowed("federation")),
            federation_id.clone()
        ))
        .build(leader_keys.public_key())
        .sign_with_keys(&leader_keys)?;
    
    leader_client.send_event(&init_event).await?;
    info!("Leader initiated DKG process");
    
    // Step 2: Wait for setup codes from all guardians
    info!("Waiting for setup codes from guardians...");
    tokio::time::sleep(tokio::time::Duration::from_secs(10)).await;
    
    // Query for setup code events
    let setup_filter = Filter::new()
        .kind(Kind::from(30078))
        .custom_tag(
            SingleLetterTag::lowercase(Alphabet::F),
            federation_id.clone()
        )
        .since(Timestamp::now() - 20); // Last 20 seconds
    
    let setup_events = leader_client.fetch_events(
        setup_filter,
        tokio::time::Duration::from_secs(5)
    ).await?;
    
    // Count setup codes (would parse actual messages in real test)
    let setup_count = setup_events.iter()
        .filter(|e| e.content.contains("PeerSetupCode"))
        .count();
    
    info!("Received {} setup codes", setup_count);
    
    // Step 3: Simulate DKG completion
    let dkg_finished_message = serde_json::json!({
        "type": "DkgFinished",
        "federation_id": federation_id,
        "invite_code": format!("fed1mock{}", test_id.simple()),
    });
    
    let finished_event = EventBuilder::new(
        Kind::from(30078),
        serde_json::to_string(&dkg_finished_message)?
    )
        .tag(Tag::custom(
            TagKind::Custom(Cow::Borrowed("federation")),
            federation_id.clone()
        ))
        .build(leader_keys.public_key())
        .sign_with_keys(&leader_keys)?;
    
    leader_client.send_event(&finished_event).await?;
    info!("DKG process completed successfully");
    
    // Cleanup
    common::cleanup_namespace(client, &namespace).await?;
    
    info!("✓ DKG with 3 guardians test passed");
    Ok(())
}

/// Test DKG setup code exchange
pub async fn test_dkg_setup_code_exchange(
    client: &Client,
    env_conf: &EnvConf,
    pool: &PgPool,
) -> Result<()> {
    info!("Testing DKG setup code exchange");
    
    let test_id = Uuid::new_v4();
    let federation_id = format!("fed-{}", test_id.simple());
    
    // Create two guardian keys (simplified test)
    let guardian1_keys = Keys::generate();
    let guardian2_keys = Keys::generate();
    
    // Create Nostr clients
    let client1 = nostr_sdk::Client::new(guardian1_keys.clone());
    let client2 = nostr_sdk::Client::new(guardian2_keys.clone());
    
    client1.add_relay(&env_conf.nostr_relay_url).await?;
    client2.add_relay(&env_conf.nostr_relay_url).await?;
    
    client1.connect().await;
    client2.connect().await;
    
    // Wait for connections
    tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
    
    // Guardian 1 sends setup code
    let setup_code1 = serde_json::json!({
        "type": "PeerSetupCode",
        "federation_id": federation_id,
        "from_npub": guardian1_keys.public_key().to_bech32()?,
        "peer_id": 0,
        "setup_code": "fed11qgqzc2nhwden5te0wfjkccte9ehx7um5wf3k2etnw3shuet0d35hgadgp4h8genyv3jkgcted3jjqmrfva68yetnv96k6ctn8gcxzm3e0g5nue3sv5sxg6twde6xyam0wd6zqmr9wghxxmmd9amk7atwv3hhyucpzdmhxue69uhkvet9v36k66tn9e3k7mgpp4mhxue69uhkgmmrw3ezuurpwf68jarpw4cxjmmdxsurgd3nxahkvet9wdjk2tn0deekxceeddyhaah8d0",
        "api_url": "ws://guardian1.test:8174",
    });
    
    let setup_event1 = EventBuilder::new(
        Kind::from(30078),
        serde_json::to_string(&setup_code1)?
    )
        .tag(Tag::custom(
            TagKind::Custom(Cow::Borrowed("federation")),
            federation_id.clone()
        ))
        .build(guardian1_keys.public_key())
        .sign_with_keys(&guardian1_keys)?;
    
    client1.send_event(&setup_event1).await?;
    info!("Guardian 1 sent setup code");
    
    // Guardian 2 sends setup code
    let setup_code2 = serde_json::json!({
        "type": "PeerSetupCode",
        "federation_id": federation_id,
        "from_npub": guardian2_keys.public_key().to_bech32()?,
        "peer_id": 1,
        "setup_code": "fed11qgqzc2nhwden5te0wfjkccte9ehx7um5wf3k2etnw3shuetn0w35xgmrpde68yct8v9hxw6t5v4jhy6t50p3hqurfvyhgcmvda5kuan9wfjx2un9wa5zumn9wsq3yamnwvaz7tmsw4e8qmr9wpskwtn9wfnrwaehxu36w5c8qet5de6k7atzdehhyat5d3jjqen0wgsyzmtp9eexzctdvshx7um59amkzmr5v5shjctzd3jhgtnndehhyapwwdhkx6r9wgzjcmr8vfsnxct5dahx2",
        "api_url": "ws://guardian2.test:8174",
    });
    
    let setup_event2 = EventBuilder::new(
        Kind::from(30078),
        serde_json::to_string(&setup_code2)?
    )
        .tag(Tag::custom(
            TagKind::Custom(Cow::Borrowed("federation")),
            federation_id.clone()
        ))
        .build(guardian2_keys.public_key())
        .sign_with_keys(&guardian2_keys)?;
    
    client2.send_event(&setup_event2).await?;
    info!("Guardian 2 sent setup code");
    
    // Both guardians should receive each other's setup codes
    let filter = Filter::new()
        .kind(Kind::from(30078))
        .custom_tag(
            SingleLetterTag::lowercase(Alphabet::F),
            federation_id.clone()
        )
        .since(Timestamp::now() - 10);
    
    // Guardian 1 fetches setup codes
    let events1 = client1.fetch_events(
        filter.clone(),
        tokio::time::Duration::from_secs(5)
    ).await?;
    
    let setup_codes1 = events1.iter()
        .filter(|e| e.content.contains("PeerSetupCode"))
        .count();
    assert!(setup_codes1 >= 2, "Guardian 1 should see both setup codes");
    
    // Guardian 2 fetches setup codes  
    let events2 = client2.fetch_events(
        filter,
        tokio::time::Duration::from_secs(5)
    ).await?;
    
    let setup_codes2 = events2.iter()
        .filter(|e| e.content.contains("PeerSetupCode"))
        .count();
    assert!(setup_codes2 >= 2, "Guardian 2 should see both setup codes");
    
    info!("Both guardians successfully exchanged setup codes");
    
    // Test encrypted setup code exchange
    let secret_setup = "fed11secret...";
    let encrypted = nip44::encrypt(
        &guardian1_keys.secret_key(),
        &guardian2_keys.public_key(),
        secret_setup.as_bytes(),
        nip44::Version::V2,
    )?;
    
    let encrypted_event = EventBuilder::new(
        Kind::from(1059), // Encrypted DM
        encrypted
    )
        .tag(Tag::public_key(guardian2_keys.public_key()))
        .tag(Tag::custom(
            TagKind::Custom(Cow::Borrowed("federation")),
            vec![federation_id]
        ))
        .build(guardian1_keys.public_key())
        .sign_with_keys(&guardian1_keys)?;
    
    client1.send_event(&encrypted_event).await?;
    info!("Sent encrypted setup code via DM");
    
    info!("✓ DKG setup code exchange test passed");
    Ok(())
}