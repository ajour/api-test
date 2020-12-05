use ajour_core::repository::curse;
use futures::future;
use isahc::prelude::*;
use serde::Serialize;

use std::collections::HashSet;
use std::time::Duration;

const CURSE_SEARCH_URL: &str = "https://addons-ecs.forgesvc.net/api/v2/addon/search";
const BATCH_SIZE: usize = 25;

#[async_std::main]
async fn main() -> Result<(), anyhow::Error> {
    let client = HttpClient::builder()
        .max_connections_per_host(3)
        .connect_timeout(Duration::from_secs(30))
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

    println!("{} packages fetched", packages.len());

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

    let curse_exact_matches = future::join_all(
        batches
            .iter()
            .map(|fingerprints| get_fingerprint_respose(&client, ApiChoice::Curse, fingerprints)),
    )
    .await
    .into_iter()
    .filter_map(|result| match result {
        Ok(info) => Some(info),
        Err(e) => {
            eprintln!("ERROR: {}", e);
            None
        }
    })
    .map(|i| i.exact_matches)
    .flatten();

    let wowup_exact_matches = future::join_all(
        batches
            .iter()
            .map(|fingerprints| get_fingerprint_respose(&client, ApiChoice::WowUp, fingerprints)),
    )
    .await
    .into_iter()
    .filter_map(|result| match result {
        Ok(info) => Some(info),
        Err(e) => {
            eprintln!("ERROR: {}", e);
            None
        }
    })
    .map(|i| i.exact_matches)
    .flatten();

    println!(
        "{} exact fingerprint matches from Curse",
        curse_exact_matches.count()
    );
    println!(
        "{} exact fingerprint matches from WowUp",
        wowup_exact_matches.count()
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

    Ok(client.send_async(request).await?.json()?)
}

enum ApiChoice {
    Curse,
    WowUp,
}

impl ApiChoice {
    const fn fingerprint_url(self) -> &'static str {
        match self {
            ApiChoice::Curse => "https://addons-ecs.forgesvc.net/api/v2/fingerprint",
            ApiChoice::WowUp => "https://hub.wowup.io/curseforge/addons/fingerprint",
        }
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
