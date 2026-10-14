use std::collections::{BTreeMap, BTreeSet};

/// Why a scan did not enable the tube.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Deny {
    Empty,
    Unknown,
    Inactive,
    Policy,
    Ambiguous,
}

impl Deny {
    pub fn message(&self, policy: &str) -> String {
        match self {
            Deny::Empty => "The reader sent an empty card id.".into(),
            Deny::Unknown => "That card is not assigned to a UniFi Access user.".into(),
            Deny::Inactive => "That user's UniFi Access account is not active.".into(),
            Deny::Policy if policy.is_empty() => "That user is not allowed to enable the laser.".into(),
            Deny::Policy => format!("That user is not on the \"{policy}\" access policy."),
            Deny::Ambiguous => "That card id matches more than one user. Laser stays locked.".into(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Holder {
    pub id: String,
    pub name: String,
    pub active: bool,
    pub policies: BTreeSet<String>,
    pub keys: BTreeSet<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Grant {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Default, Clone)]
pub struct Directory {
    holders: Vec<Holder>,
    /// Normalized form → indexes into `holders`.
    index: BTreeMap<String, Vec<usize>>,
}

impl Directory {
    pub fn from_holders(holders: Vec<Holder>) -> Self {
        let mut index: BTreeMap<String, Vec<usize>> = BTreeMap::new();
        for (i, holder) in holders.iter().enumerate() {
            for key in &holder.keys {
                for form in equivalent_forms(key) {
                    index.entry(form).or_default().push(i);
                }
            }
        }
        Self { holders, index }
    }

    pub fn len(&self) -> usize {
        self.holders.len()
    }

    pub fn is_empty(&self) -> bool {
        self.holders.is_empty()
    }

    pub fn authorize(&self, scan: &str, policy: &str) -> Result<Grant, Deny> {
        let forms = equivalent_forms(scan);
        if forms.is_empty() {
            return Err(Deny::Empty);
        }
        let mut hits: BTreeSet<usize> = BTreeSet::new();
        for form in &forms {
            if let Some(ids) = self.index.get(form) {
                hits.extend(ids.iter().copied());
            }
        }
        if hits.is_empty() {
            return Err(Deny::Unknown);
        }
        let policy_key = policy.trim().to_ascii_lowercase();
        let mut allowed: Vec<&Holder> = Vec::new();
        let mut saw_inactive = false;
        let mut saw_policy_miss = false;
        for idx in hits {
            let holder = &self.holders[idx];
            if !holder.active {
                saw_inactive = true;
                continue;
            }
            if !policy_key.is_empty() && !holder.policies.contains(&policy_key) {
                saw_policy_miss = true;
                continue;
            }
            allowed.push(holder);
        }
        let ids: BTreeSet<&str> = allowed.iter().map(|h| h.id.as_str()).collect();
        if ids.len() > 1 {
            return Err(Deny::Ambiguous);
        }
        if let Some(holder) = allowed.first() {
            return Ok(Grant {
                id: holder.id.clone(),
                name: holder.name.clone(),
            });
        }
        if saw_inactive && !saw_policy_miss {
            Err(Deny::Inactive)
        } else if saw_policy_miss {
            Err(Deny::Policy)
        } else if saw_inactive {
            Err(Deny::Inactive)
        } else {
            Err(Deny::Unknown)
        }
    }
}

/// Forms of a card id that should compare equal: case, separators, hex/decimal, byte order.
pub fn equivalent_forms(raw: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let cleaned: String = raw
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .map(|c| c.to_ascii_lowercase())
        .collect();
    if cleaned.len() < 4 || cleaned.len() > 64 {
        return out;
    }
    push_form(&mut out, &cleaned);

    if cleaned.chars().all(|c| c.is_ascii_hexdigit()) {
        if let Ok(value) = u128::from_str_radix(&cleaned, 16) {
            push_form(&mut out, &value.to_string());
            push_form(&mut out, &format!("{value:x}"));
        }
        if cleaned.len() % 2 == 0 && cleaned.len() <= 32 {
            if let Some(reversed) = reverse_hex_bytes(&cleaned) {
                push_form(&mut out, &reversed);
                if let Ok(value) = u128::from_str_radix(&reversed, 16) {
                    push_form(&mut out, &value.to_string());
                    push_form(&mut out, &format!("{value:x}"));
                }
            }
        }
    }

    if cleaned.chars().all(|c| c.is_ascii_digit()) {
        if let Ok(value) = cleaned.parse::<u128>() {
            push_form(&mut out, &format!("{value:x}"));
            push_form(&mut out, &format!("{value:08x}"));
            push_form(&mut out, &format!("{value:010x}"));
        }
    }
    out
}

fn push_form(out: &mut BTreeSet<String>, value: &str) {
    let trimmed = value.trim_start_matches('0');
    if value.len() >= 4 && value.len() <= 64 {
        out.insert(value.to_string());
    }
    if trimmed.len() >= 4 && trimmed.len() <= 64 && trimmed != value {
        out.insert(trimmed.to_string());
    }
}

fn reverse_hex_bytes(hex: &str) -> Option<String> {
    if hex.len() % 2 != 0 {
        return None;
    }
    let bytes = (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16))
        .collect::<Result<Vec<_>, _>>()
        .ok()?;
    Some(
        bytes
            .iter()
            .rev()
            .map(|b| format!("{b:02x}"))
            .collect::<String>(),
    )
}

/// Longest token on a reader line. Readers often wrap the id in a prefix.
pub fn extract_token(line: &str) -> Option<String> {
    let mut best: Option<String> = None;
    let mut current = String::new();
    let flush = |current: &mut String, best: &mut Option<String>| {
        if current.len() >= 4 && best.as_ref().map(|b| current.len() > b.len()).unwrap_or(true) {
            *best = Some(std::mem::take(current));
        } else {
            current.clear();
        }
    };
    for ch in line.chars() {
        if ch.is_ascii_alphanumeric() {
            current.push(ch);
        } else {
            flush(&mut current, &mut best);
        }
    }
    flush(&mut current, &mut best);
    best
}

#[cfg(test)]
mod tests {
    use super::*;

    fn holder(id: &str, name: &str, active: bool, policies: &[&str], keys: &[&str]) -> Holder {
        Holder {
            id: id.into(),
            name: name.into(),
            active,
            policies: policies.iter().map(|p| p.to_ascii_lowercase()).collect(),
            keys: keys.iter().map(|k| (*k).to_string()).collect(),
        }
    }

    #[test]
    fn decimal_reader_matches_hex_token() {
        let dir = Directory::from_holders(vec![holder(
            "u1",
            "Ada Lovelace",
            true,
            &["Laser"],
            &["0000DEAD"],
        )]);
        let grant = dir.authorize("57005", "Laser").unwrap();
        assert_eq!(grant.name, "Ada Lovelace");
    }

    #[test]
    fn byte_order_and_separators() {
        let dir = Directory::from_holders(vec![holder(
            "u1",
            "Grace",
            true,
            &["laser"],
            &["DE:AD:BE:EF"],
        )]);
        assert!(dir.authorize("EFBEADDE", "Laser").is_ok());
    }

    #[test]
    fn policy_and_inactive() {
        let dir = Directory::from_holders(vec![
            holder("u1", "Trainee", true, &["Members"], &["11112222"]),
            holder("u2", "Left", false, &["Laser"], &["33334444"]),
        ]);
        assert_eq!(dir.authorize("11112222", "Laser"), Err(Deny::Policy));
        assert_eq!(dir.authorize("33334444", "Laser"), Err(Deny::Inactive));
        assert!(dir.authorize("11112222", "").is_ok());
    }

    #[test]
    fn extract_longest_token() {
        assert_eq!(
            extract_token("ID=0001234567\r").as_deref(),
            Some("0001234567")
        );
    }
}
