//! P0 smoke: the built program loads into LiteSVM and the fake GameEntry deserializes.
mod common;
use {anchor_lang::prelude::Pubkey, common::*};

#[test]
fn program_loads_and_game_entry_reads() {
    let mut svm = new_svm();
    let game = Pubkey::new_unique();
    let owner = Pubkey::new_unique();
    put_game_entry(&mut svm, &game, &owner, false);
    let e: ::lobby::platform_core::accounts::GameEntry = read(&svm, &game_entry_pda(&game));
    assert_eq!(e.game_program, game);
    assert_eq!(e.owner, owner);
    assert_eq!(e.game_authority, game_authority(&game));
    assert!(matches!(e.status, ::lobby::platform_core::types::GameStatus::Active));
    assert!(svm.get_account(&::lobby::ID).unwrap().executable);
}
