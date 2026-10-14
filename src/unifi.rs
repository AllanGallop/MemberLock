use std::collections::BTreeSet;
use std::time::Duration;

use serde::Deserialize;

use crate::config::UnifiConfig;
use crate::identity::{Directory, Holder};

#[derive(Debug, Deserialize)]
struct Envelope<T> {
    code: Option<String>,
    data: Option<T>,
    msg: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct User {
    id: Option<String>,
    first_name: Option<String>,
    last_name: Option<String>,
    status: Option<String>,
    employee_number: Option<String>,
    nfc_cards: Option<Vec<Card>>,
    access_policy_ids: Option<Vec<String>>,
    access_policies: Option<Vec<Policy>>,
}

#[derive(Debug, Default, Deserialize)]
struct Card {
    id: Option<String>,
    token: Option<String>,
    alias: Option<String>,
    note: Option<String>,
    display_id: Option<String>,
    status: Option<String>,
    user: Option<CardUser>,
}

#[derive(Debug, Default, Deserialize)]
struct CardUser {
    id: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct Policy {
    id: Option<String>,
    name: Option<String>,
}

pub async fn fetch_directory(cfg: &UnifiConfig) -> anyhow::Result<Directory> {
    if cfg.base_url.is_empty() || cfg.api_token.is_empty() {
        anyhow::bail!("UniFi Access base_url and api_token are not set");
    }
    let client = reqwest::Client::builder()
        .danger_accept_invalid_certs(cfg.insecure_tls)
        .timeout(Duration::from_secs(15))
        .build()?;

    let users = get_pages::<User>(&client, cfg, "/api/v1/developer/users", true).await?;
    let cards = get_pages::<Card>(
        &client,
        cfg,
        "/api/v1/developer/credentials/nfc_cards/tokens",
        false,
    )
    .await
    .unwrap_or_default();

    Ok(Directory::from_holders(build_holders(&users, &cards)))
}

async fn get_pages<T: for<'de> Deserialize<'de>>(
    client: &reqwest::Client,
    cfg: &UnifiConfig,
    path: &str,
    expand_policy: bool,
) -> anyhow::Result<Vec<T>> {
    let mut all = Vec::new();
    for page in 1..=100 {
        let url = format!("{}{path}", cfg.base_url);
        let mut request = client
            .get(&url)
            .header("Authorization", format!("Bearer {}", cfg.api_token))
            .header("accept", "application/json")
            .query(&[
                ("page_num", page.to_string()),
                ("page_size", "100".to_string()),
            ]);
        if expand_policy {
            request = request.query(&[("expand[]", "access_policy")]);
        }
        let response = request.send().await?;
        let status = response.status();
        let body = response.text().await?;
        if !status.is_success() {
            anyhow::bail!("{path} returned HTTP {status}: {}", trim_body(&body));
        }
        let envelope: Envelope<Vec<T>> = serde_json::from_str(&body).map_err(|err| {
            anyhow::anyhow!(
                "{path} page {page} was not a list ({err}): {}",
                trim_body(&body)
            )
        })?;
        if let Some(code) = envelope.code.as_deref() {
            if !code.eq_ignore_ascii_case("SUCCESS") {
                anyhow::bail!(
                    "{path} returned {code}: {}",
                    envelope.msg.unwrap_or_default()
                );
            }
        }
        let mut batch = envelope.data.unwrap_or_default();
        let n = batch.len();
        all.append(&mut batch);
        if n < 100 {
            break;
        }
    }
    Ok(all)
}

fn trim_body(body: &str) -> String {
    let compact: String = body.chars().take(180).collect();
    compact.replace('\n', " ")
}

fn build_holders(users: &[User], cards: &[Card]) -> Vec<Holder> {
    let mut holders: Vec<Holder> = users
        .iter()
        .filter_map(|user| {
            let id = user.id.clone()?;
            let mut keys = BTreeSet::new();
            add_key(&mut keys, user.employee_number.as_deref());
            if let Some(list) = &user.nfc_cards {
                for card in list {
                    add_card_keys(&mut keys, card);
                }
            }
            let mut policies = BTreeSet::new();
            if let Some(ids) = &user.access_policy_ids {
                for policy_id in ids {
                    add_policy(&mut policies, policy_id);
                }
            }
            if let Some(list) = &user.access_policies {
                for policy in list {
                    add_policy(&mut policies, policy.id.as_deref().unwrap_or(""));
                    add_policy(&mut policies, policy.name.as_deref().unwrap_or(""));
                }
            }
            Some(Holder {
                id,
                name: display_name(user.first_name.as_deref(), user.last_name.as_deref()),
                active: is_active(user.status.as_deref()),
                policies,
                keys,
            })
        })
        .collect();

    for card in cards {
        if let Some(status) = card.status.as_deref() {
            if !status.eq_ignore_ascii_case("assigned") && !status.is_empty() {
                continue;
            }
        }
        let Some(user_id) = card.user.as_ref().and_then(|u| u.id.clone()) else {
            continue;
        };
        let Some(holder) = holders.iter_mut().find(|h| h.id == user_id) else {
            continue;
        };
        add_card_keys(&mut holder.keys, card);
    }
    holders
}

fn add_card_keys(keys: &mut BTreeSet<String>, card: &Card) {
    add_key(keys, card.token.as_deref());
    add_key(keys, card.id.as_deref());
    add_key(keys, card.display_id.as_deref());
    add_key(keys, card.alias.as_deref());
    add_key(keys, card.note.as_deref());
}

fn add_key(keys: &mut BTreeSet<String>, value: Option<&str>) {
    let Some(value) = value.map(str::trim).filter(|v| v.len() >= 4) else {
        return;
    };
    keys.insert(value.to_string());
}

fn add_policy(policies: &mut BTreeSet<String>, value: &str) {
    let value = value.trim().to_ascii_lowercase();
    if !value.is_empty() {
        policies.insert(value);
    }
}

fn display_name(first: Option<&str>, last: Option<&str>) -> String {
    let name = format!("{} {}", first.unwrap_or("").trim(), last.unwrap_or("").trim());
    let name = name.trim();
    if name.is_empty() {
        "Member".into()
    } else {
        name.to_string()
    }
}

fn is_active(status: Option<&str>) -> bool {
    match status {
        Some(status) => status.eq_ignore_ascii_case("ACTIVE"),
        // Older payloads omit status on users that are in use. Treat missing as active
        // only when the caller still has to pass the policy check.
        None => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn joins_alias_and_policy() {
        let users = vec![User {
            id: Some("u1".into()),
            first_name: Some("Ada".into()),
            last_name: Some("Lovelace".into()),
            status: Some("ACTIVE".into()),
            employee_number: None,
            nfc_cards: Some(vec![Card {
                token: Some("abc12345".into()),
                ..Card::default()
            }]),
            access_policy_ids: None,
            access_policies: Some(vec![Policy {
                id: Some("p1".into()),
                name: Some("Laser".into()),
            }]),
        }];
        let cards = vec![Card {
            alias: Some("0001234567".into()),
            status: Some("assigned".into()),
            user: Some(CardUser {
                id: Some("u1".into()),
            }),
            ..Card::default()
        }];
        let dir = Directory::from_holders(build_holders(&users, &cards));
        let grant = dir.authorize("0001234567", "Laser").unwrap();
        assert_eq!(grant.name, "Ada Lovelace");
        assert!(dir.authorize("0001234567", "Woodshop").is_err());
    }
}
