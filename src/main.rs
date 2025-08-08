use anyhow::Result;
use chrono::{DateTime, NaiveDateTime, Utc};
use solana_client::{
    rpc_client::RpcClient,
    rpc_config::RpcSignaturesForAddressConfig,
};
use solana_sdk::pubkey::Pubkey;
use solana_transaction_status::{UiInstruction, UiParsedInstruction, UiTransactionEncoding};
use std::str::FromStr;

const USDC_MINT_ADDRESS: &str = "Es9vMFrzaCERH16Cdv83hA5KaM6rDx8JEX5Rk3z3aZ9o";
const WALLET_ADDRESS: &str = "7cMEhpt9y3inBNVv8fNnuaEbx7hKHZnLvR1KWKKxuDDU";

#[tokio::main]
async fn main() -> Result<()> {
    let rpc_url = "https://api.mainnet-beta.solana.com";
    let client = RpcClient::new(rpc_url.to_string());

    let wallet = Pubkey::from_str(WALLET_ADDRESS)?;

    let now = chrono::Utc::now();
    let cutoff_ts = (now.timestamp() - 24 * 3600) as u64;

    let mut before_signature: Option<String> = None;

    let mut transfers = Vec::new();

    'outer: loop {
        let sigs = client.get_signatures_for_address_with_config(
            &wallet,
            RpcSignaturesForAddressConfig {
                before: before_signature.clone(),
                limit: Some(1000),
                ..Default::default()
            },
        )?;

        if sigs.is_empty() {
            break;
        }

        for sig_info in &sigs {
            let block_time = sig_info.block_time.unwrap_or(0) as u64;
            if block_time < cutoff_ts {
                break 'outer;
            }

            let tx = client.get_transaction(
                &sig_info.signature.parse()?,
                UiTransactionEncoding::JsonParsed,
            )?;

            if let Some(message) = &tx.transaction.transaction.message {
                for ix in &message.instructions {
                    if let UiInstruction::Parsed(UiParsedInstruction { program, parsed, .. }) = ix {
                        if program != "spl-token" {
                            continue;
                        }

                        // parsed is a serde_json::Value, parse it
                        if let Some(instruction_type) = parsed.get("type").and_then(|v| v.as_str()) {
                            if instruction_type != "transfer" && instruction_type != "transferChecked" {
                                continue;
                            }

                            let info = parsed.get("info");
                            if info.is_none() {
                                continue;
                            }
                            let info = info.unwrap();

                            // Check mint address
                            if let Some(mint) = info.get("mint").and_then(|v| v.as_str()) {
                                if mint != USDC_MINT_ADDRESS {
                                    continue;
                                }
                            }

                            let source = info.get("source").and_then(|v| v.as_str());
                            let destination = info.get("destination").and_then(|v| v.as_str());
                            let amount_str = if let Some(a) = info.get("amount").and_then(|v| v.as_str()) {
                                a.to_string()
                            } else if let Some(token_amount) = info.get("tokenAmount") {
                                token_amount.get("amount").and_then(|v| v.as_str()).unwrap_or("0").to_string()
                            } else {
                                "0".to_string()
                            };

                            let amount_u64 = amount_str.parse::<u64>().unwrap_or(0);
                            if amount_u64 == 0 {
                                continue;
                            }

                            // USDC decimals = 6, convert to float
                            let amount = amount_u64 as f64 / 1_000_000f64;

                            // Determine direction
                            let direction = if let Some(src) = source {
                                if src == WALLET_ADDRESS {
                                    "sent"
                                } else if let Some(dest) = destination {
                                    if dest == WALLET_ADDRESS {
                                        "received"
                                    } else {
                                        continue;
                                    }
                                } else {
                                    continue;
                                }
                            } else {
                                continue;
                            };

                            // Convert block_time to DateTime<Utc>
                            let date = DateTime::<Utc>::from_utc(
                                NaiveDateTime::from_timestamp(block_time as i64, 0),
                                Utc,
                            );

                            transfers.push((date, amount, direction.to_string(), sig_info.signature.clone()));
                        }
                    }
                }
            }
        }

        before_signature = sigs.last().map(|s| s.signature.clone());
    }

    // Sort by date ascending
    transfers.sort_by_key(|t| t.0);

    println!("USDC transfers for wallet: {} (last 24h)", WALLET_ADDRESS);
    for (date, amount, direction, _signature) in transfers {
        let sign = if direction == "sent" { "-" } else { "+" };
        println!("{} | {}{:.6} USDC | {}", date.to_rfc3339(), sign, amount, direction);
    }

    Ok(())
  }
                      
