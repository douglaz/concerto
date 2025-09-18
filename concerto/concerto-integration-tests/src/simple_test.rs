// Simple test to verify basic compilation

use anyhow::Result;
use nostr_sdk::prelude::*;

pub async fn simple_nostr_test() -> Result<()> {
    // Create test keys
    let keys = Keys::generate();
    
    // Create client
    let client = Client::new(keys.clone());
    
    // Add relay
    client.add_relay("ws://localhost:8008").await?;
    
    // Connect
    client.connect().await;
    
    // Create a simple event
    let event = EventBuilder::text_note("Test message")
        .build(keys.public_key())
        .sign_with_keys(&keys)?;
    
    // Send event
    let output = client.send_event(&event).await?;
    println!("Event sent with ID: {}", output.id());
    
    // Create filter
    let filter = Filter::new()
        .id(*output.id())
        .limit(1);
    
    // Fetch events
    let events = client.fetch_events(
        filter,
        std::time::Duration::from_secs(5)
    ).await?;
    
    if !events.is_empty() {
        println!("Found {} events", events.len());
    }
    
    Ok(())
}