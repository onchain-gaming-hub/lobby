//! P5 — compute units per instruction (LiteSVM runs the same SBF program; CU ≈ on-chain).
//! Prints `CU <ix> <case> <units>` (run with `--nocapture`) and asserts a generous ceiling so a
//! regression is noticed. Worst cases use max_seats = 8 (prev_room / room deserialization grows with seats).
mod common;
use {
    ::lobby::*,
    common::*,
    litesvm::types::TransactionMetadata,
    solana_signer::Signer,
};

const CEILING: u64 = 100_000;

fn cu(name: &str, case: &str, m: TransactionMetadata) {
    println!("CU {name} {case} {}", m.compute_units_consumed);
    assert!(m.compute_units_consumed < CEILING, "{name} {case}: {}", m.compute_units_consumed);
}

fn params(n: u8) -> ConfigParams {
    let mut p = default_params();
    p.max_seats = n;
    p.min_seats = n;
    p.ttl.rematch_window = 300;
    p
}

#[test]
fn measure_all() {
    for n in [4u8, 8] {
        let case = &format!("n={n}");
        let mut svm = new_svm();
        svm.add_program(GAME, &program_so("mock_game")).unwrap();
        // init_lobby / update_config
        let owner = funded(&mut svm);
        put_game_entry(&mut svm, &GAME, &owner.pubkey(), false);
        cu("init_lobby", case, ok(send(&mut svm, &[ix_init_lobby(&GAME, &owner.pubkey(), params(n))], &[&owner])));
        cu("update_config", case, ok(send(&mut svm, &[ix_update_config(&GAME, &owner.pubkey(), params(n))], &[&owner])));
        let game = GAME;

        // init_claim
        let host = funded(&mut svm);
        cu("init_claim", case, ok(send(&mut svm, &[ix_init_claim(&host.pubkey(), &host.pubkey())], &[&host])));

        // create_room (prev_room placeholder) · join_room (prev_room = a terminal room → full deserialize)
        let id = next_room_id(&svm, &game);
        cu("create_room", case, ok({ let ix = ix_create_room(&svm, &game, &host.pubkey(), &host.pubkey(), id); send(&mut svm, &[ix], &[&host]) }));
        let mut others = Vec::new();
        for i in 1..n {
            let p = player(&mut svm);
            // give the first joiner a prev_room to deserialize (worst case)
            if i == 1 {
                let x = player(&mut svm);
                let tmp = create_room(&mut svm, &game, &x);
                join(&mut svm, &game, tmp, &p);
                ok(send(&mut svm, &[ix_leave_room(&game, tmp, &p.pubkey(), false)], &[&p]));
                let m = ok({ let ix = ix_join_room(&svm, &game, id, &p.pubkey()); send(&mut svm, &[ix], &[&p]) });
                cu("join_room", &format!("{case} prev_room=live-room"), m);
            } else {
                join(&mut svm, &game, id, &p);
            }
            others.push(p);
        }
        // kick + rejoin-less: kick the last one, add bot, remove bot, add bot again
        let last = others.pop().unwrap();
        let seat = n - 1;
        cu("kick_player", case, ok(send(&mut svm, &[ix_kick(&game, id, &host.pubkey(), seat, &last.pubkey())], &[&host])));
        cu("add_bot", case, ok(send(&mut svm, &[ix_add_bot(&game, id, &host.pubkey(), seat)], &[&host])));
        cu("remove_bot", case, ok(send(&mut svm, &[ix_remove_bot(&game, id, &host.pubkey(), seat)], &[&host])));
        ok(send(&mut svm, &[ix_add_bot(&game, id, &host.pubkey(), seat)], &[&host]));
        cu("request_start", case, ok(send(&mut svm, &[ix_request_start(&game, id, &host.pubkey())], &[&host])));
        for (i, p) in others.iter().enumerate() {
            let m = ok(send(&mut svm, &[ix_ready(&game, id, &p.pubkey())], &[p]));
            if i + 1 == others.len() {
                cu("ready", &format!("{case} last→Starting"), m);
            }
        }
        cu("begin_match (via game CPI)", case, ok(crank(&mut svm, ix_start_game(&game, id))));
        cu("leave_seat", case, ok(send(&mut svm, &[ix_leave_seat(&game, id, &others[0].pubkey())], &[&others[0]])));
        advance(&mut svm, CREATE_COOLDOWN_SECS);
        cu("end_match (via game CPI)", case, ok(crank(&mut svm, ix_finish_game(&game, id, true))));
        // rematch
        let r1 = next_room_id(&svm, &game);
        cu("rematch_create", case, ok({ let ix = ix_rematch_create(&svm, &game, id, &host.pubkey(), &host.pubkey(), None, r1); send(&mut svm, &[ix], &[&host]) }));
        cu("rematch_join", case, ok({ let ix = ix_rematch_join(&svm, &game, id, &others[1].pubkey(), r1); send(&mut svm, &[ix], &[&others[1]]) }));
        cu("leave_room", case, ok(send(&mut svm, &[ix_leave_room(&game, r1, &others[1].pubkey(), false)], &[&others[1]])));
        // tick no-op · close_room · close_claim
        cu("tick", &format!("{case} no-op"), ok(crank(&mut svm, ix_tick(&game, r1, false))));
        advance(&mut svm, 600);
        let rp = room(&svm, &game, id).rent_payer;
        cu("close_room", case, ok(crank(&mut svm, ix_close_room(&game, id, &rp))));
        advance(&mut svm, CLAIM_IDLE_SECS);
        cu("close_claim", &format!("{case} prev_room=closed"), ok({ let ix = ix_close_claim(&svm, &last.pubkey(), &last.pubkey()); crank(&mut svm, ix) }));

        // public: quick_match_create / join (fill → ReadyCheck) / tick backfill / tick revert
        let q: Vec<_> = (0..n).map(|_| player(&mut svm)).collect();
        let qid = next_room_id(&svm, &game);
        cu("quick_match_create", case, ok({ let ix = ix_qm_create(&svm, &game, &q[0].pubkey(), &q[0].pubkey(), qid); send(&mut svm, &[ix], &[&q[0]]) }));
        for (i, p) in q.iter().enumerate().skip(1) {
            let m = ok({ let ix = ix_qm_join(&svm, &game, qid, &p.pubkey()); send(&mut svm, &[ix], &[p]) });
            if i + 1 == q.len() {
                cu("quick_match_join", &format!("{case} fills→ReadyCheck"), m);
            }
        }
        let b = player(&mut svm);
        let bid = next_room_id(&svm, &game);
        { let ix = ix_qm_create(&svm, &game, &b.pubkey(), &b.pubkey(), bid); ok(send(&mut svm, &[ix], &[&b])); }
        advance(&mut svm, 60);
        cu("tick", &format!("{case} public backfill"), ok(crank(&mut svm, ix_tick(&game, bid, true))));
        advance(&mut svm, 30);
        cu("tick", &format!("{case} ready-timeout cancel"), ok(crank(&mut svm, ix_tick(&game, bid, true))));
    }
}
