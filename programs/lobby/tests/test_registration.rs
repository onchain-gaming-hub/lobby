//! docs/relayer-registration.md must match idls/lobby.json — the relayer is registered from that doc.
//! (Run through scripts/test.sh so idls/lobby.json is regenerated from the current source.)
use {::lobby::*, anchor_lang::Space, std::collections::BTreeSet};

fn idl() -> serde_json::Value {
    serde_json::from_str(include_str!("../../../idls/lobby.json")).unwrap()
}
const DOC: &str = include_str!("../../../docs/relayer-registration.md");
const DEVNET_LAMPORTS_PER_BYTE: u64 = 5_080; // verified on devnet 2026-10-11 (docs/relayer-registration.md §1)

fn rent(bytes: usize) -> u64 {
    (128 + bytes as u64) * DEVNET_LAMPORTS_PER_BYTE
}

/// Rows of a markdown section whose first cell starts with a backticked name.
fn section(title: &str) -> Vec<Vec<String>> {
    let start = DOC.find(title).unwrap_or_else(|| panic!("section {title}"));
    let rest = &DOC[start + title.len()..];
    let end = rest.find("\n## ").unwrap_or(rest.len());
    rest[..end]
        .lines()
        .filter(|l| l.starts_with("| `"))
        .map(|l| l.split('|').map(|c| c.trim().to_string()).collect())
        .collect()
}

/// All backticked ix names in a first cell, e.g. "`init_lobby` · `update_config`".
fn names(cell: &str) -> Vec<String> {
    cell.split('`').skip(1).step_by(2).map(str::to_string).collect()
}

#[test]
fn sponsored_rows_match_the_idl_exactly() {
    let idl = idl();
    let ixs = idl["instructions"].as_array().unwrap();
    let rows = section("## 1.");
    assert_eq!(rows.len(), 15, "§1 must list all 15 sponsored ixs");
    for cols in rows {
        let name = names(&cols[1])[0].clone();
        let ix = ixs.iter().find(|i| i["name"] == name.as_str()).unwrap_or_else(|| panic!("{name} not in IDL"));
        let disc: String = ix["discriminator"].as_array().unwrap().iter().map(|b| format!("{:02x}", b.as_u64().unwrap())).collect();
        assert_eq!(cols[2], format!("`{disc}`"), "{name} discriminator");
        let accs = ix["accounts"].as_array().unwrap();
        // accounts cell: exactly "N" or "**N..N**" (+ an optional explanation in parentheses)
        let acc_cell = cols[3].split(" (").next().unwrap();
        let want = if name == "tick" { format!("**{0}..{0}**", accs.len() + 1) } else { accs.len().to_string() };
        assert_eq!(acc_cell, want, "{name} accounts");
        let payer = accs.iter().position(|a| a["name"] == "payer");
        assert_eq!(cols[4], payer.map_or("—".to_string(), |i| format!("**{i}**")), "{name} payer_index");
        // max_data_len: 8-byte discriminator + Borsh args; every sponsored arg must be a u8
        let args = ix["args"].as_array().unwrap();
        for a in args {
            assert_eq!(a["type"], "u8", "{name}: arg {} is not u8 — recompute max_data_len", a["name"]);
        }
        assert_eq!(cols[5], (8 + args.len()).to_string(), "{name} max_data_len");
        let want_rent = match name.as_str() {
            "init_claim" => rent(8 + PlayerClaim::INIT_SPACE),
            _ if payer.is_some() => rent(Room::space(MAX_SEATS as usize)),
            _ => 0,
        };
        let doc_rent: u64 = cols[6].trim_matches('*').replace(',', "").parse().unwrap_or_else(|_| panic!("{name} rent {:?}", cols[6]));
        assert_eq!(doc_rent, want_rent, "{name} max_rent_lamports");
    }
}

#[test]
fn sponsored_plus_not_sponsored_is_exactly_the_idl() {
    let idl: BTreeSet<String> = idl()["instructions"].as_array().unwrap().iter().map(|i| i["name"].as_str().unwrap().to_string()).collect();
    let mut doc = BTreeSet::new();
    for title in ["## 1.", "## 2."] {
        for cols in section(title) {
            for n in names(&cols[1]) {
                assert!(doc.insert(n.clone()), "{n} listed twice");
            }
        }
    }
    assert_eq!(doc, idl, "every IDL ix must be in exactly one of §1 / §2");
}

#[test]
fn min_balance_row_matches() {
    let want = 4 * rent(Room::space(MAX_SEATS as usize)) + 1_000_000;
    let row = DOC.lines().find(|l| l.contains("`min_balance_lamports`")).expect("min_balance row");
    assert!(row.contains(&format!("**{}**", want.to_string().as_bytes().rchunks(3).rev().map(|c| std::str::from_utf8(c).unwrap()).collect::<Vec<_>>().join(","))), "{row}");
}
