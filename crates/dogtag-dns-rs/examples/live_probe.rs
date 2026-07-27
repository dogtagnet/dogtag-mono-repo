//! Live probe: run the SHIPPED resolver against real DNS and print what it actually observed.
//!
//! This exists so the three DNS states can be demonstrated end to end without any fixture — every line
//! it prints is the outcome of a real resolution against a real domain, through exactly the code path
//! the government API uses.
//!
//! Usage: `cargo run -p dogtag-dns-rs --example live_probe -- <clone-addr> <domain> [doh-endpoint]`
use dogtag_dns_rs::{txt_name, BindingResolver};

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().collect();
    let clone = args.get(1).cloned().unwrap_or_default();
    let domain = args.get(2).cloned().unwrap_or_default();
    let endpoint = args
        .get(3)
        .cloned()
        .unwrap_or_else(|| "https://cloudflare-dns.com/dns-query".to_string());

    println!("query name : {:?}", txt_name(&clone, &domain));
    println!("resolver   : {endpoint}");
    let r = BindingResolver::production(endpoint);
    let c = r.check(&clone, &domain).await;
    println!("state      : {:?}", c.status);
    println!("checked_at : {}", c.checked_at);
    println!("answer_ttl : {:?}", c.answer_ttl);
}
