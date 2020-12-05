use ajour_core::repository::curse;
use anyhow::bail;
use futures::future;
use isahc::prelude::*;
use serde::Serialize;

use std::collections::HashSet;
use std::fmt::{self, Display};
use std::time::Duration;

const CURSE_SEARCH_URL: &str = "https://addons-ecs.forgesvc.net/api/v2/addon/search";
const CURSE_FINGERPRINT_URL: &str = "https://addons-ecs.forgesvc.net/api/v2/fingerprint";
const WOWUP_FINGERPRINT_URL: &str = "https://hub.wowup.io/curseforge/addons/fingerprint";
const BATCH_SIZE: usize = 25;
const MAX_HOST_CONNECTIONS: usize = 3;
const CONNECTION_TIMEOUT_SECONDS: u64 = 30;

#[async_std::main]
async fn main() -> Result<(), anyhow::Error> {
    let client = HttpClient::builder()
        .max_connections_per_host(MAX_HOST_CONNECTIONS)
        .connect_timeout(Duration::from_secs(CONNECTION_TIMEOUT_SECONDS))
        .build()?;

    let request = Request::builder()
        .method("GET")
        .uri(&format!(
            "{}?gameId=1&sort={}&pageSize={}",
            CURSE_SEARCH_URL,
            CurseSort::Popularity as u8,
            500
        ))
        .body(())
        .unwrap();

    let packages: Vec<curse::Package> = client.send_async(request).await?.json()?;

    println!("{} packages to audit against", packages.len());

    let package_fingerprints = packages
        .iter()
        .map(|p| {
            p.latest_files
                .iter()
                .map(|f| f.modules.iter().map(|m| m.fingerprint))
                .flatten()
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();

    let batches = package_fingerprints
        .chunks(BATCH_SIZE)
        .map(|batch| batch.iter().flatten().cloned().collect::<HashSet<_>>())
        .collect::<Vec<_>>();

    let curse_batches = future::join_all(
        batches
            .iter()
            .map(|fingerprints| get_fingerprint_respose(&client, ApiChoice::Curse, fingerprints)),
    );

    let wowup_batches = future::join_all(
        batches
            .iter()
            .map(|fingerprints| get_fingerprint_respose(&client, ApiChoice::WowUp, fingerprints)),
    );

    let mut responses = future::join_all(vec![curse_batches, wowup_batches]).await;

    let curse_exact_matches = responses
        .remove(0)
        .into_iter()
        .filter_map(Result::ok)
        .map(|i| i.exact_matches)
        .flatten()
        .collect::<Vec<_>>();

    let wowup_exact_matches = responses
        .remove(0)
        .into_iter()
        .filter_map(Result::ok)
        .map(|i| i.exact_matches)
        .flatten()
        .collect::<Vec<_>>();

    let unique_package_ids = [&curse_exact_matches[..], &wowup_exact_matches[..]]
        .concat()
        .into_iter()
        .map(|i| i.id)
        .collect::<HashSet<_>>();

    let curse_package_ids = curse_exact_matches
        .iter()
        .map(|i| i.id)
        .collect::<HashSet<_>>();
    let wowup_package_ids = wowup_exact_matches
        .iter()
        .map(|i| i.id)
        .collect::<HashSet<_>>();

    println!(
        "{} unique packages between both APIs",
        unique_package_ids.len(),
    );

    println!(
        "{} packages from Curse with {} fingerprint matches",
        curse_package_ids.len(),
        curse_exact_matches.len()
    );
    println!(
        "{} packages from WowUp with {} fingerprint matches",
        wowup_package_ids.len(),
        wowup_exact_matches.len()
    );

    Ok(())
}

async fn get_fingerprint_respose(
    client: &HttpClient,
    api_choice: ApiChoice,
    fingerprints: impl IntoIterator<Item = &u32>,
) -> Result<curse::FingerprintInfo, anyhow::Error> {
    let fingerprints = fingerprints.into_iter().cloned().collect::<Vec<_>>();

    let body = match api_choice {
        ApiChoice::Curse => serde_json::to_vec(&fingerprints)?,
        ApiChoice::WowUp => serde_json::to_vec(&WowUpFingerprintRequest { fingerprints })?,
    };

    let request = Request::builder()
        .uri(api_choice.fingerprint_url())
        .method("POST")
        .header("content-type", "application/json")
        .body(body)?;

    let response = client.send_async(request).await;

    match response {
        Ok(mut body) => match body.json() {
            Ok(info) => Ok(info),
            Err(e) => {
                eprintln!(
                    "ERROR: {} - failed to deserialize fingerprint request, got body: {}",
                    api_choice,
                    body.text_async().await?
                );
                bail!(e);
            }
        },
        Err(e) => {
            eprintln!("ERROR: {} - request failed: {}", api_choice, e);
            bail!(e);
        }
    }
}

enum ApiChoice {
    Curse,
    WowUp,
}

impl ApiChoice {
    const fn fingerprint_url(&self) -> &'static str {
        match self {
            ApiChoice::Curse => CURSE_FINGERPRINT_URL,
            ApiChoice::WowUp => WOWUP_FINGERPRINT_URL,
        }
    }
}

impl Display for ApiChoice {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}",
            match self {
                ApiChoice::Curse => "curse_api",
                ApiChoice::WowUp => "wowup_api",
            }
        )
    }
}

#[allow(dead_code)]
#[repr(u8)]
enum CurseSort {
    DateCreated = 1,
    LastUpdated = 2,
    Name = 3,
    Popularity = 4,
    TotalDownloads = 5,
}

#[derive(Serialize)]
struct WowUpFingerprintRequest {
    fingerprints: Vec<u32>,
}
