//! 06-relayer-registration.md must match idls/lobby.json — the relayer is registered from that doc.
use {::lobby::*, anchor_lang::Space};

fn idl() -> serde_json::Value {
    serde_json::from_str(include_str!("../../../idls/lobby.json")).unwrap()
}
const DOC: &str = include_str!("../../../specs/designs/06-relayer-registration.md");
const DEVNET_LAMPORTS_PER_BYTE: u64 = 5_080; // verified 2026-10-11 (06 §1)

fn rent(bytes: usize) -> u64 {
    (128 + bytes as u64) * DEVNET_LAMPORTS_PER_BYTE
}

#[test]
fn sponsored_rows_match_the_idl() {
    let idl = idl();
    let ixs = idl["instructions"].as_array().unwrap();
    let mut rows = 0;
    for line in DOC.lines().filter(|l| l.starts_with("| `")) {
        let cols: Vec<&str> = line.split('|').map(str::trim).collect();
        // | `ix` [(crank)] | `disc` | accounts | payer_index | max_data_len | max_rent | CU |
        if cols.len() < 8 || !cols[2].starts_with('`') {
            continue;
        }
        let name = cols[1].trim_start_matches('`').split('`').next().unwrap();
        let Some(ix) = ixs.iter().find(|i| i["name"] == name) else { continue };
        rows += 1;
        let disc: String = ix["discriminator"].as_array().unwrap().iter().map(|b| format!("{:02x}", b.as_u64().unwrap())).collect();
        assert_eq!(cols[2].trim_matches('`'), disc, "{name} discriminator");
        let accs = ix["accounts"].as_array().unwrap();
        let n = accs.len();
        let crank = name == "tick";
        let want_accounts = if crank { format!("**{0}..{0}**", n + 1) } else { n.to_string() };
        assert!(cols[3].starts_with(&want_accounts), "{name} accounts: doc {:?} want {want_accounts}", cols[3]);
        let payer = accs.iter().position(|a| a["name"] == "payer");
        match payer {
            Some(i) => assert_eq!(cols[4], format!("**{i}**"), "{name} payer_index"),
            None => assert_eq!(cols[4], "—", "{name} payer_index"),
        }
        let data_len = 8 + ix["args"].as_array().unwrap().len(); // all sponsored args are 1-byte (u8)
        assert_eq!(cols[5], data_len.to_string(), "{name} max_data_len");
        let want_rent = match name {
            "init_claim" => rent(8 + PlayerClaim::INIT_SPACE),
            _ if payer.is_some() => rent(Room::space(MAX_SEATS as usize)),
            _ => 0,
        };
        let doc_rent: u64 = cols[6].trim_matches('*').replace(',', "").parse().unwrap_or_else(|_| panic!("{name} rent {:?}", cols[6]));
        assert_eq!(doc_rent, want_rent, "{name} max_rent_lamports");
    }
    assert_eq!(rows, 15, "06 must list all 15 sponsored ixs");
}

#[test]
fn min_balance_matches() {
    let want = 4 * rent(Room::space(MAX_SEATS as usize)) + 1_000_000;
    assert_eq!(want, 13_578_080);
    assert!(DOC.contains("13,578,080"));
}
