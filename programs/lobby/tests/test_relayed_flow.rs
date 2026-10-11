//! The whole private-room flow as a gasless frontend sends it: new players (no claim yet) send
//! `[init_claim, enter]` in ONE transaction, and a relayer is both fee payer and rent payer.
mod common;
use {
    ::lobby::*,
    anchor_lang::{InstructionData, Space, ToAccountMetas},
    anchor_lang::solana_program::instruction::Instruction,
    common::*,
    solana_signer::Signer,
};

#[test]
fn new_players_create_join_ready_and_play_with_a_relayer_paying_everything() {
    // a 2-player head-to-head game: private rooms only, no bots, generous start/game windows
    let params = ConfigParams {
        max_seats: 2,
        min_seats: 2,
        min_humans: 2,
        public_enabled: false,
        private_enabled: true,
        bot_fill: false,
        rematch_enabled: false,
        ttl: RoomTtl { wait: 60, ready: 120, lobby: 1800, start: 300, max_game: 7200, rematch_window: 60, retention: 600 },
    };
    let mut svm = new_svm();
    svm.add_program(GAME, &program_so("mock_game")).unwrap();
    let game = setup_game(&mut svm, GAME, params).program;
    let relayer = funded(&mut svm);
    let (a, b) = (funded(&mut svm), funded(&mut svm));
    let (a_before, b_before) = (lamports(&svm, &a.pubkey()), lamports(&svm, &b.pubkey()));
    let relayer_before = lamports(&svm, &relayer.pubkey());

    // A: [init_claim, create_room] — claim doesn't exist yet, so prev_room = placeholder (program ID)
    let id = next_room_id(&svm, &game);
    let create = Instruction {
        program_id: ::lobby::ID,
        accounts: ::lobby::accounts::CreateRoom {
            player: a.pubkey(),
            payer: relayer.pubkey(),
            game_entry: game_entry_pda(&game),
            config: config_pda(&game),
            matchmaker: matchmaker_pda(&game),
            claim: claim_pda(&a.pubkey()),
            prev_room: None,
            new_room: room_pda(&game, id),
            system_program: anchor_lang::system_program::ID,
        }
        .to_account_metas(None),
        data: ::lobby::instruction::CreateRoom {}.data(),
    };
    ok(send(&mut svm, &[ix_init_claim(&a.pubkey(), &relayer.pubkey()), create], &[&relayer, &a]));

    // B: [init_claim, join_room]
    let join = Instruction {
        program_id: ::lobby::ID,
        accounts: ::lobby::accounts::JoinRoom { player: b.pubkey(), room: room_pda(&game, id), claim: claim_pda(&b.pubkey()), prev_room: None }
            .to_account_metas(None),
        data: ::lobby::instruction::JoinRoom {}.data(),
    };
    ok(send(&mut svm, &[ix_init_claim(&b.pubkey(), &relayer.pubkey()), join], &[&relayer, &b]));

    // host starts (host is auto-ready) → B ready → Starting → game CPI begin_match → Playing → end_match → Ended
    ok(send(&mut svm, &[ix_request_start(&game, id, &a.pubkey())], &[&relayer, &a]));
    assert_eq!(room(&svm, &game, id).phase, Phase::ReadyCheck);
    ok(send(&mut svm, &[ix_ready(&game, id, &b.pubkey())], &[&relayer, &b]));
    assert_eq!(room(&svm, &game, id).phase, Phase::Starting);
    ok(send(&mut svm, &[ix_start_game(&game, id)], &[&relayer]));
    assert_eq!(room(&svm, &game, id).phase, Phase::Playing);
    ok(send(&mut svm, &[ix_finish_game(&game, id, true)], &[&relayer]));
    assert_eq!(room(&svm, &game, id).phase, Phase::Ended);

    // players never paid a lamport · the relayer is rent_payer of both claims and the room
    assert_eq!(lamports(&svm, &a.pubkey()), a_before);
    assert_eq!(lamports(&svm, &b.pubkey()), b_before);
    let r = room(&svm, &game, id);
    assert_eq!(r.rent_payer, relayer.pubkey());
    for p in [&a, &b] {
        let c: PlayerClaim = read(&svm, &claim_pda(&p.pubkey()));
        assert_eq!(c.rent_payer, relayer.pubkey());
    }
    let rents = 2 * svm.minimum_balance_for_rent_exemption(8 + PlayerClaim::INIT_SPACE)
        + svm.minimum_balance_for_rent_exemption(Room::space(params.max_seats as usize));
    assert!(relayer_before - lamports(&svm, &relayer.pubkey()) >= rents, "relayer fronted the rent");
}
