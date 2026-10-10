//! Shared LiteSVM harness. platform-core `GameEntry` is written directly (owner = platform-core ID,
//! layout from idls/platform_core.json) — lobby only *reads* it, so platform-core is not loaded.
#![allow(dead_code, clippy::result_large_err, clippy::too_many_arguments)]

use {
    ::lobby::*,
    anchor_lang::{prelude::Pubkey, AccountDeserialize},
    litesvm::{types::TransactionResult, LiteSVM},
    solana_account::Account,
    solana_keypair::Keypair,
    solana_message::{Message, VersionedMessage},
    solana_signer::Signer,
    solana_transaction::versioned::VersionedTransaction,
};

pub const SOL: u64 = 1_000_000_000;
pub const PC: Pubkey = ::lobby::platform_core::ID;
const GAME_ENTRY_DISC: [u8; 8] = [119, 212, 18, 192, 14, 188, 27, 149];

pub fn program_so(name: &str) -> Vec<u8> {
    let path = format!("{}/../deploy/{name}.so", env!("CARGO_TARGET_TMPDIR"));
    std::fs::read(&path).unwrap_or_else(|e| panic!("{path}: {e} — build first: ./scripts/test.sh"))
}

pub fn pda(seeds: &[&[u8]], program: &Pubkey) -> Pubkey {
    Pubkey::find_program_address(seeds, program).0
}
pub fn game_authority(game: &Pubkey) -> Pubkey {
    pda(&[GAME_AUTHORITY_SEED], game)
}
pub fn game_entry_pda(game: &Pubkey) -> Pubkey {
    pda(&[PC_GAME_SEED, game.as_ref()], &PC)
}
pub fn config_pda(game: &Pubkey) -> Pubkey {
    pda(&[LOBBY_CONFIG_SEED, game.as_ref()], &::lobby::ID)
}
pub fn matchmaker_pda(game: &Pubkey) -> Pubkey {
    pda(&[MATCHMAKER_SEED, game.as_ref()], &::lobby::ID)
}
pub fn room_pda(game: &Pubkey, room_id: u64) -> Pubkey {
    pda(&[ROOM_SEED, game.as_ref(), &room_id.to_le_bytes()], &::lobby::ID)
}
pub fn claim_pda(player: &Pubkey) -> Pubkey {
    pda(&[CLAIM_SEED, player.as_ref()], &::lobby::ID)
}

pub fn put(svm: &mut LiteSVM, addr: Pubkey, owner: Pubkey, data: Vec<u8>) {
    let lamports = svm.minimum_balance_for_rent_exemption(data.len());
    svm.set_account(addr, Account { lamports, data, owner, executable: false, rent_epoch: 0 }).unwrap();
}

/// platform-core `GameEntry` v1 (01-account-layout-v1 §4 · 220 B).
pub fn put_game_entry(svm: &mut LiteSVM, game: &Pubkey, owner: &Pubkey, suspended: bool) {
    let (ga, ga_bump) = Pubkey::find_program_address(&[GAME_AUTHORITY_SEED], game);
    let bump = Pubkey::find_program_address(&[PC_GAME_SEED, game.as_ref()], &PC).1;
    let mut d = GAME_ENTRY_DISC.to_vec();
    d.extend([1, bump, suspended as u8]);
    d.extend(game.as_ref());
    d.extend(owner.as_ref());
    d.extend(ga.as_ref());
    d.push(ga_bump);
    d.extend(Pubkey::new_unique().as_ref()); // registered_by
    d.extend(0i64.to_le_bytes());
    d.extend(0i64.to_le_bytes());
    d.extend([0u8; 64]);
    assert_eq!(d.len(), 220);
    put(svm, game_entry_pda(game), PC, d);
}

pub fn funded(svm: &mut LiteSVM) -> Keypair {
    let kp = Keypair::new();
    svm.airdrop(&kp.pubkey(), 100 * SOL).unwrap();
    kp
}

pub fn send(svm: &mut LiteSVM, ixs: &[anchor_lang::solana_program::instruction::Instruction], signers: &[&Keypair]) -> TransactionResult {
    let bh = svm.latest_blockhash();
    let msg = Message::new_with_blockhash(ixs, Some(&signers[0].pubkey()), &bh);
    let tx = VersionedTransaction::try_new(VersionedMessage::Legacy(msg), signers).unwrap();
    let res = svm.send_transaction(tx);
    svm.expire_blockhash();
    res
}

pub fn ok(res: TransactionResult) -> litesvm::types::TransactionMetadata {
    res.unwrap_or_else(|e| panic!("tx failed: {:?}\n{:#?}", e.err, e.meta.logs))
}

pub fn assert_err(res: &TransactionResult, code: u32) {
    let e = res.as_ref().expect_err("expected failure");
    let s = format!("{:?}", e.err);
    assert!(s.contains(&format!("Custom({code})")), "expected Custom({code}), got {s}\n{:#?}", e.meta.logs);
}

pub fn lamports(svm: &LiteSVM, a: &Pubkey) -> u64 {
    svm.get_account(a).map_or(0, |x| x.lamports)
}
pub fn gone(svm: &LiteSVM, a: &Pubkey) -> bool {
    svm.get_account(a).is_none_or(|x| x.data.is_empty() && x.lamports == 0)
}

pub fn read<T: AccountDeserialize>(svm: &LiteSVM, a: &Pubkey) -> T {
    let acc = svm.get_account(a).unwrap_or_else(|| panic!("account {a} missing"));
    T::try_deserialize(&mut &acc.data[..]).unwrap()
}

pub fn set_time(svm: &mut LiteSVM, unix: i64) {
    let mut c: anchor_lang::prelude::Clock = svm.get_sysvar();
    c.unix_timestamp = unix;
    svm.set_sysvar(&c);
}
pub fn now(svm: &LiteSVM) -> i64 {
    let c: anchor_lang::prelude::Clock = svm.get_sysvar();
    c.unix_timestamp
}

pub fn new_svm() -> LiteSVM {
    let mut svm = LiteSVM::new();
    svm.add_program(::lobby::ID, &program_so("lobby")).unwrap();
    set_time(&mut svm, 1_760_000_000);
    svm
}

// ─────────────────── instruction builders ───────────────────
use anchor_lang::{solana_program::instruction::Instruction, InstructionData, ToAccountMetas};

pub fn code(e: ::lobby::errors::LobbyError) -> u32 {
    anchor_lang::error::ERROR_CODE_OFFSET + e as u32
}

pub fn default_params() -> ConfigParams {
    ConfigParams {
        max_seats: 4,
        min_seats: 4,
        min_humans: 1,
        public_enabled: true,
        private_enabled: true,
        bot_fill: true,
        rematch_enabled: true,
        ttl: RoomTtl { wait: 60, ready: 30, lobby: 1800, start: 60, max_game: 3600, rematch_window: 60, retention: 600 },
    }
}

pub fn ix_init_lobby(game: &Pubkey, owner: &Pubkey, params: ConfigParams) -> Instruction {
    Instruction {
        program_id: ::lobby::ID,
        accounts: ::lobby::accounts::InitLobby {
            owner: *owner,
            game_program: *game,
            game_entry: game_entry_pda(game),
            config: config_pda(game),
            matchmaker: matchmaker_pda(game),
            system_program: anchor_lang::system_program::ID,
        }
        .to_account_metas(None),
        data: ::lobby::instruction::InitLobby { params }.data(),
    }
}

pub fn ix_update_config(game: &Pubkey, owner: &Pubkey, params: ConfigParams) -> Instruction {
    Instruction {
        program_id: ::lobby::ID,
        accounts: ::lobby::accounts::UpdateConfig { owner: *owner, game_entry: game_entry_pda(game), config: config_pda(game) }
            .to_account_metas(None),
        data: ::lobby::instruction::UpdateConfig { params }.data(),
    }
}

pub fn ix_init_claim(player: &Pubkey, payer: &Pubkey) -> Instruction {
    Instruction {
        program_id: ::lobby::ID,
        accounts: ::lobby::accounts::InitClaim {
            player: *player,
            payer: *payer,
            claim: claim_pda(player),
            system_program: anchor_lang::system_program::ID,
        }
        .to_account_metas(None),
        data: ::lobby::instruction::InitClaim {}.data(),
    }
}

/// A game registered in platform-core (Active) with its lobby initialised by `owner`.
pub struct Game {
    pub program: Pubkey,
    pub owner: Keypair,
}

pub fn setup_game(svm: &mut LiteSVM, program: Pubkey, params: ConfigParams) -> Game {
    let owner = funded(svm);
    put_game_entry(svm, &program, &owner.pubkey(), false);
    ok(send(svm, &[ix_init_lobby(&program, &owner.pubkey(), params)], &[&owner]));
    Game { program, owner }
}

// ─────────────────── P2: rooms ───────────────────

/// The room the player's claim points at (what clients pass as `prev_room`).
pub fn prev_of(svm: &LiteSVM, player: &Pubkey) -> Option<Pubkey> {
    let c: PlayerClaim = read(svm, &claim_pda(player));
    c.current.map(|r| room_pda(&r.game_program, r.room_id))
}

/// A funded player with a claim (paid by themselves).
pub fn player(svm: &mut LiteSVM) -> Keypair {
    let p = funded(svm);
    ok(send(svm, &[ix_init_claim(&p.pubkey(), &p.pubkey())], &[&p]));
    p
}

pub fn next_room_id(svm: &LiteSVM, game: &Pubkey) -> u64 {
    let m: Matchmaker = read(svm, &matchmaker_pda(game));
    m.next_room_id
}

pub fn ix_create_room(svm: &LiteSVM, game: &Pubkey, player: &Pubkey, payer: &Pubkey, room_id: u64) -> Instruction {
    Instruction {
        program_id: ::lobby::ID,
        accounts: ::lobby::accounts::CreateRoom {
            player: *player,
            payer: *payer,
            game_entry: game_entry_pda(game),
            config: config_pda(game),
            matchmaker: matchmaker_pda(game),
            claim: claim_pda(player),
            prev_room: prev_of(svm, player),
            new_room: room_pda(game, room_id),
            system_program: anchor_lang::system_program::ID,
        }
        .to_account_metas(None),
        data: ::lobby::instruction::CreateRoom {}.data(),
    }
}

/// Create a private room as `host` (payer = host). Returns room_id.
pub fn create_room(svm: &mut LiteSVM, game: &Pubkey, host: &Keypair) -> u64 {
    let id = next_room_id(svm, game);
    ok({ let ix = ix_create_room(svm, game, &host.pubkey(), &host.pubkey(), id); send(svm, &[ix], &[host]) });
    id
}

pub fn ix_join_room(svm: &LiteSVM, game: &Pubkey, room_id: u64, player: &Pubkey) -> Instruction {
    Instruction {
        program_id: ::lobby::ID,
        accounts: ::lobby::accounts::JoinRoom {
            player: *player,
            config: config_pda(game),
            room: room_pda(game, room_id),
            claim: claim_pda(player),
            prev_room: prev_of(svm, player),
        }
        .to_account_metas(None),
        data: ::lobby::instruction::JoinRoom {}.data(),
    }
}

pub fn ix_leave_room(game: &Pubkey, room_id: u64, player: &Pubkey, public: bool) -> Instruction {
    Instruction {
        program_id: ::lobby::ID,
        accounts: ::lobby::accounts::LeaveRoom {
            player: *player,
            room: room_pda(game, room_id),
            matchmaker: public.then(|| matchmaker_pda(game)),
        }
        .to_account_metas(None),
        data: ::lobby::instruction::LeaveRoom {}.data(),
    }
}

pub fn ix_ready(game: &Pubkey, room_id: u64, player: &Pubkey) -> Instruction {
    Instruction {
        program_id: ::lobby::ID,
        accounts: ::lobby::accounts::Ready { player: *player, room: room_pda(game, room_id) }.to_account_metas(None),
        data: ::lobby::instruction::Ready {}.data(),
    }
}

fn host_metas(game: &Pubkey, room_id: u64, host: &Pubkey) -> Vec<anchor_lang::prelude::AccountMeta> {
    ::lobby::accounts::HostRoom { host: *host, room: room_pda(game, room_id) }.to_account_metas(None)
}
pub fn ix_add_bot(game: &Pubkey, room_id: u64, host: &Pubkey, seat: u8) -> Instruction {
    Instruction { program_id: ::lobby::ID, accounts: host_metas(game, room_id, host), data: ::lobby::instruction::AddBot { seat }.data() }
}
pub fn ix_remove_bot(game: &Pubkey, room_id: u64, host: &Pubkey, seat: u8) -> Instruction {
    Instruction { program_id: ::lobby::ID, accounts: host_metas(game, room_id, host), data: ::lobby::instruction::RemoveBot { seat }.data() }
}
pub fn ix_request_start(game: &Pubkey, room_id: u64, host: &Pubkey) -> Instruction {
    Instruction { program_id: ::lobby::ID, accounts: host_metas(game, room_id, host), data: ::lobby::instruction::RequestStart {}.data() }
}
pub fn ix_kick(game: &Pubkey, room_id: u64, host: &Pubkey, seat: u8, victim: &Pubkey) -> Instruction {
    Instruction {
        program_id: ::lobby::ID,
        accounts: ::lobby::accounts::KickPlayer { host: *host, room: room_pda(game, room_id), victim_claim: claim_pda(victim) }
            .to_account_metas(None),
        data: ::lobby::instruction::KickPlayer { seat }.data(),
    }
}

pub fn room(svm: &LiteSVM, game: &Pubkey, room_id: u64) -> Room {
    read(svm, &room_pda(game, room_id))
}

pub fn join(svm: &mut LiteSVM, game: &Pubkey, room_id: u64, p: &Keypair) {
    ok({ let ix = ix_join_room(svm, game, room_id, &p.pubkey()); send(svm, &[ix], &[p]) });
}

// ─────────────────── P3: game CPI · crank · close ───────────────────

pub const GAME: Pubkey = mock_game::ID;

/// LiteSVM with lobby + mock-game loaded and mock-game registered (Active) + lobby initialised.
pub fn game_world() -> (LiteSVM, Game) {
    let mut svm = new_svm();
    svm.add_program(GAME, &program_so("mock_game")).unwrap();
    let g = setup_game(&mut svm, GAME, default_params());
    (svm, g)
}

fn play_metas(game: &Pubkey, room_id: u64) -> Vec<anchor_lang::prelude::AccountMeta> {
    mock_game::accounts::Play {
        room: room_pda(game, room_id),
        game_entry: game_entry_pda(game),
        game_authority: game_authority(&GAME),
        lobby_program: ::lobby::ID,
    }
    .to_account_metas(None)
}
pub fn ix_start_game(game: &Pubkey, room_id: u64) -> Instruction {
    Instruction { program_id: GAME, accounts: play_metas(game, room_id), data: mock_game::instruction::StartGame {}.data() }
}
pub fn ix_finish_game(game: &Pubkey, room_id: u64, completed: bool) -> Instruction {
    Instruction { program_id: GAME, accounts: play_metas(game, room_id), data: mock_game::instruction::FinishGame { completed }.data() }
}
/// Call lobby directly with an arbitrary signer as game_authority (attack path).
pub fn ix_begin_match_direct(game: &Pubkey, room_id: u64, signer: &Pubkey) -> Instruction {
    Instruction {
        program_id: ::lobby::ID,
        accounts: ::lobby::accounts::GameCpi { room: room_pda(game, room_id), game_entry: game_entry_pda(game), game_authority: *signer }
            .to_account_metas(None),
        data: ::lobby::instruction::BeginMatch {}.data(),
    }
}
pub fn ix_leave_seat(game: &Pubkey, room_id: u64, player: &Pubkey) -> Instruction {
    Instruction {
        program_id: ::lobby::ID,
        accounts: ::lobby::accounts::LeaveSeat { player: *player, room: room_pda(game, room_id) }.to_account_metas(None),
        data: ::lobby::instruction::LeaveSeat {}.data(),
    }
}
pub fn ix_tick(game: &Pubkey, room_id: u64, public: bool) -> Instruction {
    Instruction {
        program_id: ::lobby::ID,
        accounts: ::lobby::accounts::Tick { room: room_pda(game, room_id), matchmaker: public.then(|| matchmaker_pda(game)) }
            .to_account_metas(None),
        data: ::lobby::instruction::Tick {}.data(),
    }
}
pub fn ix_close_room(game: &Pubkey, room_id: u64, rent_payer: &Pubkey) -> Instruction {
    Instruction {
        program_id: ::lobby::ID,
        accounts: ::lobby::accounts::CloseRoom { room: room_pda(game, room_id), rent_payer: *rent_payer }.to_account_metas(None),
        data: ::lobby::instruction::CloseRoom {}.data(),
    }
}
pub fn ix_close_claim(svm: &LiteSVM, player: &Pubkey, rent_payer: &Pubkey) -> Instruction {
    Instruction {
        program_id: ::lobby::ID,
        accounts: ::lobby::accounts::CloseClaim { claim: claim_pda(player), rent_payer: *rent_payer, prev_room: prev_of(svm, player) }
            .to_account_metas(None),
        data: ::lobby::instruction::CloseClaim {}.data(),
    }
}

/// host + one more human + 2 bots → request_start → other ready → Starting. Returns (room_id, host, other).
pub fn starting_room(svm: &mut LiteSVM, game: &Pubkey) -> (u64, Keypair, Keypair) {
    let host = player(svm);
    let other = player(svm);
    let id = create_room(svm, game, &host);
    join(svm, game, id, &other);
    ok(send(svm, &[ix_add_bot(game, id, &host.pubkey(), 2)], &[&host]));
    ok(send(svm, &[ix_add_bot(game, id, &host.pubkey(), 3)], &[&host]));
    ok(send(svm, &[ix_request_start(game, id, &host.pubkey())], &[&host]));
    ok(send(svm, &[ix_ready(game, id, &other.pubkey())], &[&other]));
    assert_eq!(room(svm, game, id).phase, Phase::Starting);
    (id, host, other)
}

pub fn advance(svm: &mut LiteSVM, secs: i64) {
    let t = now(svm);
    set_time(svm, t + secs);
}

pub fn crank(svm: &mut LiteSVM, ix: Instruction) -> TransactionResult {
    let k = funded(svm);
    send(svm, &[ix], &[&k])
}
