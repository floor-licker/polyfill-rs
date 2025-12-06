use polyfill_rs::ClobClient;
use std::time::Instant;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    dotenv::dotenv().ok();

    println!("🔄 Connection Reuse Test");
    println!("========================\n");

    let api_key = std::env::var("POLYMARKET_API_KEY")?;
    let secret = std::env::var("POLYMARKET_SECRET")?;
    let passphrase = std::env::var("POLYMARKET_PASSPHRASE")?;

    let api_creds = polyfill_rs::ApiCredentials {
        api_key,
        secret,
        passphrase,
    };

    let mut client = ClobClient::new("https://clob.polymarket.com");
    client.set_api_creds(api_creds);

    println!("Making 10 sequential requests (should reuse connection)...\n");

    let mut times = Vec::new();

    for i in 1..=10 {
        let start = Instant::now();
        
        let response = client.http_client
            .get(format!("{}/sampling-markets?next_cursor=MA==", client.base_url))
            .send()
            .await?;
        
        let status = response.status();
        let _json: serde_json::Value = response.json().await?;
        
        let elapsed = start.elapsed();
        times.push(elapsed);
        
        println!("Request {:2}: {:>6.1} ms (status: {})", 
            i, 
            elapsed.as_micros() as f64 / 1000.0,
            status.as_u16()
        );
    }

    println!("\n📊 Analysis:");
    println!("===========");
    
    let first = times[0];
    let avg_rest: std::time::Duration = times[1..].iter().sum::<std::time::Duration>() / (times.len() - 1) as u32;
    
    println!("First request:  {:.1} ms (includes connection setup)", first.as_micros() as f64 / 1000.0);
    println!("Avg subsequent: {:.1} ms (should reuse connection)", avg_rest.as_micros() as f64 / 1000.0);
    
    let improvement = ((first.as_micros() as f64 - avg_rest.as_micros() as f64) / first.as_micros() as f64) * 100.0;
    
    if improvement > 20.0 {
        println!("\n✅ Connection reuse is working! ({:.0}% faster)", improvement);
    } else if improvement > 5.0 {
        println!("\n⚠️  Some connection reuse, but not optimal ({:.0}% improvement)", improvement);
    } else {
        println!("\n❌ Connection reuse NOT working (only {:.0}% improvement)", improvement);
        println!("   Expected: 30-50% improvement on subsequent requests");
    }

    Ok(())
}
