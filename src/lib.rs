//! WebAssembly (WASM) entry point for the Defender game.
//!
//! This file exposes a small C-compatible interface (`extern "C"`) so that
//! browser JavaScript (`web/main.js`) can drive the Rust game engine without
//! needing any third-party binding libraries.
//!
//! ## Key Rust Concepts Used Here
//! - `pub mod game`: Declares and publicly exports the `game` module (`src/game.rs`).
//! - `thread_local!` + `RefCell<Game>`: Rust forbids mutable global variables
//!   (`static mut`) in safe code to prevent data races. Because WebAssembly is
//!   single-threaded, storing the `Game` inside a `thread_local!` `RefCell` lets
//!   us safely borrow it immutably (`.borrow()`) or mutably (`.borrow_mut()`)
//!   with runtime borrow checking instead of `unsafe` code.
//! - `#[no_mangle]` + `pub extern "C" fn`: Tells the Rust compiler to keep the
//!   exact function name (instead of mangling it) and use the standard C calling
//!   convention so JavaScript can call it directly from `instance.exports`.

pub mod game;

use std::cell::RefCell;

use game::{Game, InputState, SCREEN_HEIGHT, SCREEN_WIDTH};

// Global game instance stored per-thread (WASM runs on a single thread).
// `RefCell` provides "interior mutability": we can mutate the `Game` inside
// even though `GAME` itself is accessed through an immutable reference.
thread_local! {
    static GAME: RefCell<Game> = RefCell::new(Game::new());
}

/// Initializes a fresh game session.
///
/// Called once by JavaScript after loading `defender.wasm`.
#[no_mangle]
pub extern "C" fn init() {
    // `.with(|g| ...)` passes a reference to the thread-local `RefCell<Game>`
    // into a closure (anonymous function). `.borrow_mut()` grants exclusive
    // mutable access so we can reset the game state.
    GAME.with(|g| g.borrow_mut().reset());
}

/// Resets the game back to tick 0 (used when the player clicks Restart or presses `R`).
#[no_mangle]
pub extern "C" fn reset() {
    GAME.with(|g| g.borrow_mut().reset());
}

/// Updates the current keyboard state from a 5-bit integer bitmask sent by JavaScript.
///
/// Bit layout:
/// - Bit 0 (`1`):  Fire laser (`Space`)
/// - Bit 1 (`2`):  Thrust left (`ArrowLeft` / `A`)
/// - Bit 2 (`4`):  Climb (`ArrowUp` / `W`)
/// - Bit 3 (`8`):  Thrust right (`ArrowRight` / `D`)
/// - Bit 4 (`16`): Dive (`ArrowDown` / `S`)
#[no_mangle]
pub extern "C" fn set_input(bits: u32) {
    GAME.with(|g| g.borrow_mut().set_input(InputState::from_bits(bits)));
}

/// Advances the game simulation and renders the next frame into the RGBA framebuffer.
///
/// JavaScript calls this at a fixed 16ms cadence (~62.5 Hz) to match the
/// original `setInterval(..., 16)` loop.
#[no_mangle]
pub extern "C" fn tick() {
    GAME.with(|g| g.borrow_mut().tick());
}

/// Returns a raw pointer (byte offset inside WASM linear memory) to the start
/// of the RGBA framebuffer.
///
/// JavaScript wraps `(wasm.memory.buffer, framebuffer_ptr(), framebuffer_len())`
/// in a `Uint8ClampedArray` to blit pixels directly onto the `<canvas>` with zero copies.
#[no_mangle]
pub extern "C" fn framebuffer_ptr() -> *const u8 {
    GAME.with(|g| g.borrow().framebuffer.as_ptr())
}

/// Returns the total size of the RGBA framebuffer in bytes (`width * height * 4`).
#[no_mangle]
pub extern "C" fn framebuffer_len() -> usize {
    GAME.with(|g| g.borrow().framebuffer.len())
}

/// Returns the canvas width in pixels (`510`).
#[no_mangle]
pub extern "C" fn width() -> u32 {
    SCREEN_WIDTH as u32
}

/// Returns the canvas height in pixels (`376`: `56`-px radar HUD + `320`-px playfield).
#[no_mangle]
pub extern "C" fn height() -> u32 {
    SCREEN_HEIGHT as u32
}

/// Returns the player's current score.
#[no_mangle]
pub extern "C" fn score() -> u32 {
    GAME.with(|g| g.borrow().score)
}

/// Returns the player's remaining lives (starts at `3`; game ends when `0`).
#[no_mangle]
pub extern "C" fn lives() -> u32 {
    GAME.with(|g| g.borrow().lives)
}

/// Returns the player's remaining Smart Bombs (starts at `3`, plus `+1` every `10,000` points).
#[no_mangle]
pub extern "C" fn smart_bombs() -> u32 {
    GAME.with(|g| g.borrow().smart_bombs)
}

/// Detonates a Smart Bomb, destroying all visible on-screen enemies and bullets.
///
/// Does nothing if the game is over, no Smart Bombs remain, or the player is missing/exploding.
#[no_mangle]
pub extern "C" fn trigger_smart_bomb() {
    GAME.with(|g| g.borrow_mut().trigger_smart_bomb());
}

/// Activates Hyperspace, instantly teleporting the player to a random location on the planet.
///
/// Unlimited uses whenever the game is active and the player is alive (`warp_explosion == 0`).
#[no_mangle]
pub extern "C" fn trigger_hyperspace() {
    GAME.with(|g| g.borrow_mut().trigger_hyperspace());
}

/// Returns the number of living humans currently on the planet or being abducted.
#[no_mangle]
pub extern "C" fn humans_alive() -> u32 {
    GAME.with(|g| g.borrow().humans_alive())
}

/// Returns the number of active enemies (Landers and Mutants) not currently exploding.
#[no_mangle]
pub extern "C" fn enemies_alive() -> u32 {
    GAME.with(|g| g.borrow().enemies_alive())
}

/// Returns `1` if the game is over, or `0` while still playing.
///
/// WebAssembly C-ABI functions return integers rather than Rust `bool` directly;
/// `u32::from(bool)` converts `false -> 0` and `true -> 1`.
#[no_mangle]
pub extern "C" fn is_game_over() -> u32 {
    GAME.with(|g| u32::from(g.borrow().game_over))
}

/// Returns the total number of simulation ticks elapsed since the last reset.
#[no_mangle]
pub extern "C" fn ticks() -> u32 {
    GAME.with(|g| g.borrow().ticks)
}

/// Returns the current camera X world coordinate (`H` in the original JS1K code).
#[no_mangle]
pub extern "C" fn camera_x() -> f64 {
    GAME.with(|g| g.borrow().camera_x)
}

/// Returns the player's facing direction (`+1` = right, `-1` = left).
#[no_mangle]
pub extern "C" fn direction() -> i32 {
    GAME.with(|g| g.borrow().direction)
}

/// Returns `1` if the player's laser beam is active on the current frame, `0` otherwise.
#[no_mangle]
pub extern "C" fn is_firing() -> u32 {
    GAME.with(|g| u32::from(g.borrow().firing))
}

/// Returns the total number of entities currently in the simulation (player, humans, enemies, bullets).
#[no_mangle]
pub extern "C" fn entity_count() -> u32 {
    GAME.with(|g| g.borrow().entities.len() as u32)
}

/// Returns the player's current X coordinate, or `0.0` if no entities exist yet.
///
/// `.first()` returns `Option<&Entity>` (`None` when the list is empty), and
/// `.map_or(0.0, |e| e.x)` safely extracts `e.x` or falls back to `0.0`.
#[no_mangle]
pub extern "C" fn player_x() -> f64 {
    GAME.with(|g| g.borrow().entities.first().map_or(0.0, |e| e.x))
}

/// Returns the player's current Y coordinate, or `0.0` if no entities exist yet.
#[no_mangle]
pub extern "C" fn player_y() -> f64 {
    GAME.with(|g| g.borrow().entities.first().map_or(0.0, |e| e.y))
}
