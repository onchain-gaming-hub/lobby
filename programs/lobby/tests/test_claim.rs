//! P1 — init_claim (02 §3.2) · rent safety R1 / R2 / R4 (03 §2).
mod common;
use {::lobby::*, anchor_lang::Space, common::*, solana_signer::Signer};

#[test]
fn init_claim_fields_and_payer_pays_exactly_rent() {
    let mut svm = new_svm();
    let player = funded(&mut svm);
    let payer = funded(&mut svm); // e.g. the relayer — NOT the fee payer here, so its delta is rent only
    let before = lamports(&svm, &payer.pubkey());
    ok(send(&mut svm, &[ix_init_claim(&player.pubkey(), &payer.pubkey())], &[&player, &payer]));
    let size = 8 + PlayerClaim::INIT_SPACE;
    assert_eq!(size, 188);
    // R4: payer's lamports drop by exactly the rent of the account it created
    assert_eq!(before - lamports(&svm, &payer.pubkey()), svm.minimum_balance_for_rent_exemption(size));
    let c: PlayerClaim = read(&svm, &claim_pda(&player.pubkey()));
    assert_eq!(c.version, 1);
    assert_eq!(c.rent_payer, payer.pubkey()); // R2: from the payer signer
    assert_eq!(c.player, player.pubkey());
    assert_eq!(c.last_joined_at, now(&svm));
    assert_eq!((c.current, c.kick), (None, None));
}

#[test]
fn r1_init_claim_twice_fails_and_rent_payer_never_changes() {
    let mut svm = new_svm();
    let player = funded(&mut svm);
    let relayer = funded(&mut svm);
    let other = funded(&mut svm);
    ok(send(&mut svm, &[ix_init_claim(&player.pubkey(), &relayer.pubkey())], &[&player, &relayer]));
    let r = send(&mut svm, &[ix_init_claim(&player.pubkey(), &other.pubkey())], &[&player, &other]);
    assert_err(&r, 0); // System AccountAlreadyInUse — plain init cannot re-init
    let c: PlayerClaim = read(&svm, &claim_pda(&player.pubkey()));
    assert_eq!(c.rent_payer, relayer.pubkey());
}

#[test]
fn claim_is_bound_to_the_signing_player() {
    let mut svm = new_svm();
    let player = funded(&mut svm);
    let payer = funded(&mut svm);
    let victim = funded(&mut svm);
    // try to create the victim's claim while only `player` signs → seeds use player.key() → wrong PDA
    let mut ix = ix_init_claim(&player.pubkey(), &payer.pubkey());
    ix.accounts[2].pubkey = claim_pda(&victim.pubkey());
    let r = send(&mut svm, &[ix], &[&player, &payer]);
    assert!(format!("{:?}", r.unwrap_err().err).contains("Custom(2006)"), "ConstraintSeeds");
}

#[test]
fn r4_prefunded_claim_pda_payer_covers_only_the_shortfall() {
    let mut svm = new_svm();
    let player = funded(&mut svm);
    let relayer = funded(&mut svm);
    svm.airdrop(&claim_pda(&player.pubkey()), 1_000_000).unwrap(); // griefer pre-funds (≥ rent of 0 B)
    let before = lamports(&svm, &relayer.pubkey());
    ok(send(&mut svm, &[ix_init_claim(&player.pubkey(), &relayer.pubkey())], &[&player, &relayer]));
    assert_eq!(before - lamports(&svm, &relayer.pubkey()), svm.minimum_balance_for_rent_exemption(188) - 1_000_000);
    let c: PlayerClaim = read(&svm, &claim_pda(&player.pubkey()));
    assert_eq!(c.rent_payer, relayer.pubkey());
}
