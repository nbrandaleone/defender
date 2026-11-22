//! Core **Defender: 1024** game engine, reverse-engineered from Keith Clark's
//! 2015 JS1K competition entry.
//!
//! This module contains:
//! - All game constants and data types (`EntityKind`, `InputState`, `Entity`, `Game`)
//! - The frame-by-frame simulation loop (`Game::tick`)
//! - A software pixel renderer writing directly into an RGBA8888 framebuffer
//! - Helper functions (`to_i32`, `hsl_to_rgb`) matching JavaScript's exact math
//! - Unit tests (`#[cfg(test)] mod tests`) verifying gameplay and parity
//!
//! ## How the Game Works
//! - The planet is a **1024-pixel wrapping horizontal world**: moving past `1023`
//!   wraps seamlessly back to `0` using bitwise AND (`& 1023`, since `1023 == 0x3FF`).
//! - The visible viewport is `255 × 160` logical pixels, drawn as `2 × 2` blocks
//!   into a `510 × 320` RGBA pixel buffer.
//! - Each entity has a 7×8 bitmap sprite stored in `SPRITE_DATA` and a single
//!   integer counter `warp_explosion` (`w` in JS) that drives both warp-in
//!   animations (`w > 0`) and pixel-scatter explosions (`w < 0`).

/// Total horizontal width of the wrapping planet world (in logical pixels).
pub const WORLD_WIDTH: i32 = 1024;
/// Bitmask (`1024 - 1 = 0b11_1111_1111`) used to wrap X coordinates into `0..=1023`.
/// Because `1024` is a power of two, `x & 1023` is equivalent to `x.rem_euclid(1024)`.
pub const WORLD_MASK: i32 = 1023;
/// Logical viewport width in game pixels (original JS checks `X < 255`).
pub const LOGICAL_WIDTH: usize = 255;
/// Logical viewport height in game pixels.
pub const LOGICAL_HEIGHT: usize = 160;
/// Each logical pixel is rendered as a `2 × 2` square on screen (`X << 1, Y << 1, 2, 2`).
pub const PIXEL_SCALE: usize = 2;
/// Final framebuffer width in screen pixels (`255 * 2 = 510`).
pub const SCREEN_WIDTH: usize = LOGICAL_WIDTH * PIXEL_SCALE;
/// Final framebuffer height in screen pixels (`160 * 2 = 320`).
pub const SCREEN_HEIGHT: usize = LOGICAL_HEIGHT * PIXEL_SCALE;
/// Total byte size of the RGBA framebuffer (4 bytes per pixel: Red, Green, Blue, Alpha).
pub const FRAMEBUFFER_SIZE: usize = SCREEN_WIDTH * SCREEN_HEIGHT * 4;

/// 7×8 pixel bitmaps for all 5 entity types, XOR-encoded with `69` (`0x45`, ASCII `'E'`).
///
/// How decoding works:
/// - The slice (`&[u8]`) holds 35 ASCII bytes: 7 columns (`V = 0..7`) for each of the
///   5 entity types (`Player`, `Human`, `Bullet`, `Lander`, `Mutant`).
/// - For a given entity `t` and column `V`, `69 ^ SPRITE_DATA[7 * t + V]` produces an
///   8-bit vertical column mask where bit `U` (`0..8`) is `1` if that pixel is lit.
/// - Because `69 ^ b'E' == 0`, `'E'` represents an empty column (giving Lander and
///   Mutant their symmetrical 5-pixel-wide jellyfish silhouette).
const SPRITE_DATA: &[u8] = b"[y}}U]UMA8Z8AMEMYYMEEE{V:V{EE{V:V{E";

/// The five kinds of entities in the game.
///
/// Rust notes for newcomers:
/// - `#[derive(...)]` automatically generates common traits:
///   - `Debug`: lets us print the enum with `{:?}` in tests/logs.
///   - `Clone, Copy`: allows cheap value-by-value copying (just 1 byte) instead of moving.
///   - `PartialEq, Eq`: allows comparing two values with `==` and `!=`.
/// - `#[repr(u8)]` guarantees each variant is stored as an explicit 8-bit integer (`0..=4`),
///   matching the original JS `t` type numbers so `self as usize` works directly.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum EntityKind {
    /// The player's Defender ship (`t = 0`, red).
    Player = 0,
    /// A surface human to protect from abduction (`t = 1`, green).
    Human = 1,
    /// An enemy projectile fired by Landers and Mutants (`t = 2`, cyan).
    Bullet = 2,
    /// An alien Lander that hunts and abducts humans (`t = 3`, magenta).
    Lander = 3,
    /// A fast, aggressive Mutant created when a human is abducted to the top (`t = 4`, orange).
    Mutant = 4,
}

impl EntityKind {
    /// Converts a JS1K spawn code (`L` = 1..=5) into an `EntityKind`.
    ///
    /// Returns `Option<Self>`: `Some(kind)` for valid codes `1..=5`, or `None` for
    /// `0` / unknown codes. Rust uses `Option` instead of `null` so callers must
    /// explicitly handle the "no value" case.
    pub fn from_spawn_code(code: u8) -> Option<Self> {
        match code {
            1 => Some(Self::Player),
            2 => Some(Self::Human),
            3 => Some(Self::Bullet),
            4 => Some(Self::Lander),
            5 => Some(Self::Mutant),
            _ => None,
        }
    }

    /// Converts this enum variant into a `usize` array index (`0..=4`).
    pub fn as_usize(self) -> usize {
        self as usize
    }

    /// Returns the movement speed multiplier (`g = L / 4` in the original JS).
    ///
    /// - Player: `0.25`
    /// - Human:  `0.50`
    /// - Bullet: `0.75`
    /// - Lander: `1.00`
    /// - Mutant: `1.25`
    pub fn speed(self) -> f64 {
        (self as u8 + 1) as f64 / 4.0
    }
}

/// Snapshot of which player control keys are currently held down.
///
/// `#[derive(Default)]` automatically implements `InputState::default()` with
/// all `bool` fields set to `false` (no keys pressed).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct InputState {
    /// Fire laser (`Space`).
    pub fire: bool,
    /// Thrust left (`ArrowLeft` / `A`).
    pub left: bool,
    /// Climb upward (`ArrowUp` / `W`).
    pub up: bool,
    /// Thrust right (`ArrowRight` / `D`).
    pub right: bool,
    /// Dive downward (`ArrowDown` / `S`).
    pub down: bool,
}

impl InputState {
    /// Decodes a 5-bit integer bitmask into an `InputState`.
    ///
    /// Bit layout: `bit0 = fire`, `bit1 = left`, `bit2 = up`, `bit3 = right`, `bit4 = down`.
    /// Using a single `u32` bitmask keeps the JavaScript <-> WASM boundary simple.
    pub fn from_bits(bits: u32) -> Self {
        Self {
            fire: (bits & 1) != 0,
            left: (bits & 2) != 0,
            up: (bits & 4) != 0,
            right: (bits & 8) != 0,
            down: (bits & 16) != 0,
        }
    }
}

/// A single active object in the game world (player, human, bullet, lander, or mutant).
#[derive(Debug, Clone)]
pub struct Entity {
    /// Unique monotonic identifier assigned at spawn time.
    ///
    /// In JavaScript, entities link to each other via direct object references (`s`).
    /// In Rust, storing a numeric `id` instead of a reference/index keeps ownership
    /// simple and prevents stale indices when dead entities are removed from `Vec<Entity>`.
    pub id: u32,
    /// Which type of entity this is (`t` in JS).
    pub kind: EntityKind,
    /// Speed multiplier applied to velocity each tick (`g = L / 4`).
    pub speed: f64,
    /// Animation / lifecycle state counter (`w` in JS):
    /// - `> 0`: Warping into the world (pixels converge from wide spacing down to 1×).
    /// - `== 0`: Normal active gameplay state.
    /// - `< 0`: Exploding (pixels fly apart until `< -25`, then the entity is removed).
    pub warp_explosion: i32,
    /// Optional ID of a linked partner entity (`s` in JS):
    /// - On a **Lander**: the `Human` it is currently chasing or abducting.
    /// - On a **Human**: the `Lander` currently lifting it skyward.
    pub target_id: Option<u32>,
    /// Horizontal velocity (`u` in JS).
    pub vx: f64,
    /// Vertical velocity (`v` in JS).
    pub vy: f64,
    /// General-purpose countdown timer (`z` in JS):
    /// - Player: laser fire duration & cooldown.
    /// - Bullet: remaining lifetime ticks.
    /// - Lander / Mutant: ticks until next bullet is fired.
    pub timer: i32,
    /// World X coordinate (`x` in JS, wraps modulo `1024`).
    pub x: f64,
    /// World Y coordinate (`y` in JS, `0` = top of screen, `140` = ground level).
    pub y: f64,
}

/// Complete state of a running Defender game, including its pixel framebuffer.
pub struct Game {
    /// Camera X world coordinate (`H` in JS); leads slightly ahead of the player's ship.
    pub camera_x: f64,
    /// Frame counter incremented once per tick (`$` in JS).
    pub ticks: u32,
    /// `true` while the player's rainbow laser beam is active (`J` in JS).
    pub firing: bool,
    /// Player facing direction (`D` in JS: `+1` = right, `-1` = left).
    pub direction: i32,
    /// Active world wrap bitmask (`A` in JS: `1023` while playing, `0` on game over).
    pub world_mask: i32,
    /// `true` once the player has finished exploding and the game has frozen.
    pub game_over: bool,
    /// Player score (awarded for destroying Landers, Mutants, and intercepting Bullets).
    pub score: u32,
    /// Dynamic list (`Vec`) of all entities (`E` in JS). Index `0` is always the Player while alive.
    pub entities: Vec<Entity>,
    /// Currently pressed controls (`W` in JS).
    pub input: InputState,
    /// Counter used to hand out unique `Entity::id` values (private field: no `pub`).
    next_entity_id: u32,
    /// Persistent spawn coordinates (`O`, `N` in JS) reused across entity spawns.
    pub spawn_x: f64,
    pub spawn_y: f64,
    /// Flat RGBA8888 pixel buffer (`SCREEN_WIDTH * SCREEN_HEIGHT * 4` bytes)
    /// rendered directly in Rust and copied to the HTML `<canvas>` each frame.
    pub framebuffer: Vec<u8>,
}

/// Implementing `Default` lets callers construct a `Game` via `Game::default()`.
impl Default for Game {
    fn default() -> Self {
        Self::new()
    }
}

impl Game {
    /// Allocates and initializes a new `Game` at tick `0`.
    pub fn new() -> Self {
        Self {
            camera_x: 0.0,
            ticks: 0,
            firing: false,
            direction: 1,
            world_mask: WORLD_MASK,
            game_over: false,
            score: 0,
            // Pre-allocate capacity for 64 entities to avoid early heap reallocations.
            entities: Vec::with_capacity(64),
            input: InputState::default(),
            next_entity_id: 1,
            spawn_x: 0.0,
            spawn_y: 0.0,
            // `vec![0; N]` creates a `Vec<u8>` filled with `N` zero bytes.
            framebuffer: vec![0; FRAMEBUFFER_SIZE],
        }
    }

    /// Resets all game state back to the beginning of a fresh game.
    ///
    /// Notice `&mut self`: in Rust, methods that modify the struct must take a
    /// mutable reference (`&mut self`), while read-only methods take `&self`.
    pub fn reset(&mut self) {
        self.camera_x = 0.0;
        self.ticks = 0;
        self.firing = false;
        self.direction = 1;
        self.world_mask = WORLD_MASK;
        self.game_over = false;
        self.score = 0;
        self.entities.clear();
        self.input = InputState::default();
        self.next_entity_id = 1;
        self.spawn_x = 0.0;
        self.spawn_y = 0.0;
        self.clear_screen();
    }

    /// Replaces the current keyboard input state.
    pub fn set_input(&mut self, input: InputState) {
        self.input = input;
    }

    /// Counts how many living humans remain (neither dead nor currently exploding).
    ///
    /// Uses Rust's iterator pipeline: `.iter()` borrows each entity, `.filter(...)`
    /// keeps only matching elements, and `.count()` tallies them without allocating.
    pub fn humans_alive(&self) -> u32 {
        self.entities
            .iter()
            .filter(|e| e.kind == EntityKind::Human && e.warp_explosion >= 0)
            .count() as u32
    }

    /// Counts how many active or warping-in enemies (Landers and Mutants) remain.
    ///
    /// `matches!(val, Pat1 | Pat2)` is a concise Rust macro that returns `true`
    /// when `val` matches any of the listed enum patterns.
    pub fn enemies_alive(&self) -> u32 {
        self.entities
            .iter()
            .filter(|e| {
                matches!(e.kind, EntityKind::Lander | EntityKind::Mutant) && e.warp_explosion >= 0
            })
            .count() as u32
    }

    /// Creates a new entity from a JS1K spawn code (`1..=5`) and appends it to `self.entities`.
    fn spawn_entity(&mut self, spawn_code: u8, x: f64, y: f64) {
        // `let Some(kind) = ... else { return; }` is Rust's "let-else" pattern:
        // unwrap the `Option`, or exit early if `from_spawn_code` returned `None`.
        let Some(kind) = EntityKind::from_spawn_code(spawn_code) else {
            return;
        };
        let id = self.next_entity_id;
        self.next_entity_id += 1;
        self.entities.push(Entity {
            id,
            kind,
            speed: kind.speed(),
            // Landers (4) and Mutants (5) warp in over 25 frames; others start active (0).
            warp_explosion: if spawn_code > 3 { 25 } else { 0 },
            target_id: None,
            vx: 0.0,
            vy: 0.0,
            timer: 0,
            x,
            y,
        });
    }

    /// Fills the entire RGBA framebuffer with opaque black `(0, 0, 0, 255)`.
    fn clear_screen(&mut self) {
        // `chunks_exact_mut(4)` yields mutable 4-byte slices `[R, G, B, A]` for each pixel,
        // allowing the compiler to optimize away per-element bounds checks.
        for px in self.framebuffer.chunks_exact_mut(4) {
            px[0] = 0;
            px[1] = 0;
            px[2] = 0;
            px[3] = 255;
        }
    }

    /// Plots a `2 × 2` pixel block at logical coordinate `(lx, ly)` with the given `(r, g, b)` color.
    ///
    /// Coordinates outside the `255 × 160` logical viewport are safely ignored.
    fn plot_pixel(&mut self, lx: i32, ly: i32, rgb: (u8, u8, u8)) {
        if lx < 0 || ly < 0 || lx >= LOGICAL_WIDTH as i32 || ly >= LOGICAL_HEIGHT as i32 {
            return;
        }
        // Convert logical coordinate to top-left screen pixel coordinate (2× scale).
        let sx = (lx as usize) * PIXEL_SCALE;
        let sy = (ly as usize) * PIXEL_SCALE;
        for dy in 0..PIXEL_SCALE {
            let row = (sy + dy) * SCREEN_WIDTH;
            for dx in 0..PIXEL_SCALE {
                // Each pixel occupies 4 consecutive bytes: [R, G, B, A].
                let idx = (row + sx + dx) * 4;
                self.framebuffer[idx] = rgb.0;
                self.framebuffer[idx + 1] = rgb.1;
                self.framebuffer[idx + 2] = rgb.2;
                self.framebuffer[idx + 3] = 255;
            }
        }
    }

    /// Advances the game by one simulation tick (~16ms) and renders the frame.
    ///
    /// This mirrors the single `for (d = A; d--; )` loop in the original JS1K game:
    /// in each of the 1023 iterations (`d = 1022` down to `0`), it draws one column
    /// of mountains, one parallax star, one laser segment, updates & draws entity `E[d]`
    /// (if present), and spawns any queued new entities.
    pub fn tick(&mut self) {
        if self.world_mask == 0 {
            // In JS1K, once the player finishes exploding, `A` becomes `0`.
            // `c.fillRect(0, 0, A, A)` draws a 0×0 rectangle, leaving the final
            // frame frozen on screen instead of clearing to black.
            self.game_over = true;
            // `wrapping_add` wraps from `u32::MAX` back to `0` instead of panicking in debug mode.
            self.ticks = self.ticks.wrapping_add(1);
            return;
        }

        self.clear_screen();

        let loop_count = self.world_mask;
        // `F` in JS: running Y height of the procedural mountain range (starts at 127).
        let mut mountain_y: f64 = 127.0;
        // `L` in JS: non-zero when an entity should be spawned at the end of this `d` step.
        let mut spawn_code: u8 = 0;

        // Iterate `d` in reverse from `1022` down to `0` (`(0..1023).rev()`).
        // Reverse order ensures:
        // 1. Removing dead entities with `entities.remove(idx)` does not shift lower indices.
        // 2. The Player (`E[0]`) is always processed last (at `d == 0`).
        for d in (0..loop_count).rev() {
            // --- 1. Procedural Mountain Pixel ---
            // `X = (d - H) & A`: subtract in floating point first, then truncate & wrap
            // to match JavaScript's `ToInt32` bitwise-AND semantics.
            let m_x = to_i32(d as f64 - self.camera_x) & self.world_mask;
            // Deterministic terrain slope: `F += Math.cos((d / 5) & -11)`.
            // Bitwise `& -11` clears bits 1 and 3, creating jagged plateau-like ridges.
            mountain_y += (((d / 5) & -11) as f64).cos();
            if m_x < 255 {
                let color = hsl_to_rgb(25.0, 0.99, 0.50); // Warm orange-brown
                self.plot_pixel(m_x, to_i32(mountain_y), color);
            }

            // --- 2. Parallax Starfield Pixel ---
            // Divides mountain X by 2 for 2× slower scrolling parallax, and hashes
            // `mountain_y` into a pseudo-random vertical coordinate `Y < 99`.
            let star_x = (m_x as f64) / 2.0;
            let star_y = to_i32(199.0 * mountain_y) & self.world_mask;
            if star_y < 99 && star_x < 255.0 {
                let color = hsl_to_rgb(-mountain_y, 0.99, 0.50); // Dark blue / violet hues
                self.plot_pixel(to_i32(star_x), star_y, color);
            }

            // --- 3. Player Rainbow Laser Pixel ---
            // While `firing` (`z > 9`), draws 25 spaced dots (`d = 0..25`) stretching
            // forward in the player's facing direction, shifting hue from red to green.
            if self.firing && d < 25 {
                if let Some(player) = self.entities.first() {
                    let laser_x = to_i32(
                        ((16 - player.timer) * d * self.direction) as f64 - self.camera_x
                            + 127.0
                            + player.x,
                    ) & self.world_mask;
                    let laser_y = to_i32(player.y);
                    if laser_x < 255 {
                        let color = hsl_to_rgb((5 * d) as f64, 0.99, 0.50);
                        self.plot_pixel(laser_x, laser_y, color);
                    }
                }
            }

            // --- 4. Entity Simulation & Sprite Rendering ---
            let idx = d as usize;
            if idx < self.entities.len() {
                // Rust's borrow checker does not allow holding a mutable borrow to
                // `self.entities[idx]` while simultaneously mutating *another* element
                // in `self.entities`. We record cross-entity actions in these small local
                // variables and apply them right after updating `self.entities[idx]`.
                let mut remove_entity = false;
                let mut kill_player = false;
                let mut unlink_partner_id: Option<u32> = None;
                let mut link_human: Option<(u32, u32)> = None;

                let entity_id = self.entities[idx].id;
                let entity_kind = self.entities[idx].kind;

                // Apply velocity and tick down timer BEFORE reading player position
                // (so when `idx == 0`, `player_x - ex == 0` even at high speeds).
                {
                    let e = &mut self.entities[idx];
                    e.x += e.vx * e.speed;
                    e.y += e.vy * e.speed;
                    if e.timer > 0 {
                        e.timer -= 1;
                    }
                }

                let warp_exp = self.entities[idx].warp_explosion;
                if warp_exp != 0 {
                    // Entity is either warping in (`w > 0`) or exploding (`w < 0`).
                    // `.take()` replaces `target_id` with `None` and returns its previous value,
                    // breaking any Lander <-> Human abduction link.
                    unlink_partner_id = self.entities[idx].target_id.take();
                    let prev_w = self.entities[idx].warp_explosion;
                    self.entities[idx].warp_explosion -= 1;
                    // Once an explosion passes frame `-25`, remove the entity (and end
                    // the game if it was the Player).
                    if prev_w < -25 {
                        if entity_kind == EntityKind::Player {
                            self.world_mask = 0;
                            self.game_over = true;
                        }
                        remove_entity = true;
                    }
                } else {
                    // Entity is active in play (`w == 0`).
                    // Compute shortest wrapped vector `(q, r)`, distance `p`, and angle `n`
                    // from this entity to the Player (`E[0]`).
                    let (player_x, player_y) = (self.entities[0].x, self.entities[0].y);
                    let ex = self.entities[idx].x;
                    let ey = self.entities[idx].y;
                    // Shifting by +511, masking `& 1023`, and subtracting 511 maps the
                    // horizontal delta into `[-511, +512]` across the 1024-px world seam.
                    let q = -511.0 + ((to_i32(player_x - ex + 511.0) & self.world_mask) as f64);
                    let r = player_y - ey;
                    let p = (q * q + r * r).sqrt();
                    let n = r.atan2(q);

                    // Collision check vs Player and vs Player's laser beam.
                    if p < 5.0 {
                        // Any non-Player, non-Human entity (`Bullet`, `Lander`, `Mutant`)
                        // within 5 logical pixels destroys the Player.
                        if entity_kind.as_usize() > 1 {
                            kill_player = true;
                        }
                    } else if self.firing
                        && r > -4.0
                        && r < 4.0
                        && p < 130.0
                        && q * (self.direction as f64) < 1.0
                    {
                        // Entity is vertically aligned (`|r| < 4`), within laser range (`p < 130`),
                        // and in front of the ship (`q * D < 1` because `q = player_x - entity_x`).
                        self.entities[idx].warp_explosion -= 1;
                        match entity_kind {
                            EntityKind::Lander => self.score += 150,
                            EntityKind::Mutant => self.score += 200,
                            EntityKind::Bullet => self.score += 50,
                            _ => {}
                        }
                    }

                    // Per-entity-type AI and controls.
                    match entity_kind {
                        EntityKind::Player => {
                            let e = &mut self.entities[idx];
                            // Camera leads ahead of the ship in its travel direction.
                            self.camera_x = e.x + e.vx * 2.0;
                            // `input_x`: `+1` for right, `-1` for left, `0` for neither/both.
                            let input_x = (!self.input.left as i32) - (!self.input.right as i32);
                            if input_x != 0 {
                                self.direction = input_x;
                            }
                            if input_x != 0 {
                                // Accelerate up to max horizontal velocity `25.0`.
                                if e.vx * (input_x as f64) < 25.0 {
                                    e.vx += (input_x as f64) / 2.0;
                                }
                            } else {
                                // Smooth inertial drag (`vx *= 0.96`) when coasting.
                                e.vx -= e.vx / 25.0;
                            }
                            // Vertical movement clamped to `0.0..=140.0`.
                            if self.input.down {
                                if e.y < 140.0 {
                                    e.y += 1.0;
                                }
                            } else if e.y != 0.0 && self.input.up {
                                e.y -= 1.0;
                            }
                            // Fire laser when cooldown reaches 0 (active for 7 frames: `16..=10`).
                            if e.timer == 0 && self.input.fire {
                                e.timer = 16;
                            }
                            self.firing = e.timer > 9;
                        }
                        EntityKind::Human => {
                            let e = &mut self.entities[idx];
                            // If grabbed by a Lander (`target_id.is_some()`), climb upward;
                            // otherwise fall gently back to ground level (`y = 140`).
                            e.vy = if e.target_id.is_some() {
                                -1.0
                            } else if e.y < 140.0 {
                                1.0
                            } else {
                                0.0
                            };
                            if e.y < 1.0 {
                                // Abducted human reached the top of the screen: kill human
                                // and queue a Mutant spawn (`L = 5`) using persistent `O, N`.
                                spawn_code = 5;
                                e.warp_explosion -= 1;
                            }
                        }
                        EntityKind::Bullet => {
                            let e = &mut self.entities[idx];
                            if e.timer == 0 {
                                if (e.vx + e.vy) != 0.0 {
                                    // Lifetime expired: destroy bullet immediately (`w = -A`).
                                    e.warp_explosion = -self.world_mask;
                                } else {
                                    // Newly spawned bullet: aim toward the player for 99 ticks.
                                    e.vx = n.cos();
                                    e.vy = n.sin();
                                    e.timer = 99;
                                }
                            }
                        }
                        EntityKind::Lander | EntityKind::Mutant => {
                            // Default behavior: steer toward the player and fire bullets periodically.
                            {
                                let e = &mut self.entities[idx];
                                e.vx = n.cos();
                                e.vy = n.sin();
                                if e.timer == 0 {
                                    spawn_code = 3; // Queue a Bullet at this enemy's position
                                    self.spawn_x = e.x;
                                    self.spawn_y = e.y;
                                    e.timer = 99 + d;
                                }
                            }

                            // Check whether this Lander is tracking a living, ungrabbed Human.
                            // `.and_then(...)` runs the closure only if `current_target` is `Some(tid)`.
                            let current_target = self.entities[idx].target_id;
                            let valid_human = current_target.and_then(|tid| {
                                self.entities.iter().find(|h| {
                                    h.id == tid
                                        && h.kind == EntityKind::Human
                                        && h.warp_explosion == 0
                                        && (h.target_id.is_none() || h.target_id == Some(entity_id))
                                })
                            });

                            if let Some(human) = valid_human {
                                // Steer toward a point 8 logical pixels directly above the human.
                                let hx = human.x;
                                let hy = human.y;
                                let hid = human.id;
                                let hq = -511.0
                                    + ((to_i32(hx - ex + 511.0) & self.world_mask) as f64);
                                let hr = hy - 8.0 - ey;
                                let hp = (hq * hq + hr * hr).sqrt();
                                let hn = hr.atan2(hq);

                                let e = &mut self.entities[idx];
                                e.vx = hn.cos();
                                e.vy = hn.sin();
                                if hp < 1.0 {
                                    // Close enough to grab: snap into alignment and link the human
                                    // to this lander so the human begins ascending.
                                    e.x = hx;
                                    e.y -= hr;
                                    link_human = Some((hid, entity_id));
                                }
                            } else if entity_kind == EntityKind::Lander && d > 0 {
                                // Not tracking a human yet: sample an entity index `ticks % d`
                                // and lock on if it happens to be a Human (Mutants never abduct).
                                let pick_idx = (self.ticks as usize) % (d as usize);
                                if let Some(cand) = self.entities.get(pick_idx) {
                                    if cand.kind == EntityKind::Human {
                                        self.entities[idx].target_id = Some(cand.id);
                                    } else {
                                        self.entities[idx].target_id = None;
                                    }
                                } else {
                                    self.entities[idx].target_id = None;
                                }
                            } else {
                                self.entities[idx].target_id = None;
                            }
                        }
                    }
                }

                // Apply deferred cross-entity updates now that `self.entities[idx]` is no longer borrowed.
                if let Some(pid) = unlink_partner_id {
                    if let Some(partner) = self.entities.iter_mut().find(|e| e.id == pid) {
                        partner.target_id = None;
                    }
                }
                if let Some((hid, lid)) = link_human {
                    if let Some(human) = self.entities.iter_mut().find(|e| e.id == hid) {
                        human.target_id = Some(lid);
                    }
                }
                if kill_player && !self.entities.is_empty() {
                    // Matches JS1K: `E[0].w--` unconditionally on every overlapping enemy tick.
                    self.entities[0].warp_explosion -= 1;
                }

                // --- Render Entity Sprite (7×8 bitmap) ---
                // `scale = w + 1`:
                // - `w == 0` -> `scale = 1` (normal crisp sprite)
                // - `w > 0`  -> `scale > 1` (warping in: pixels converge inward)
                // - `w < 0`  -> `scale <= 0` (exploding: pixels scatter outward)
                let (ex, ey, ew, ekind) = {
                    let e = &self.entities[idx];
                    (e.x, e.y, e.warp_explosion, e.kind)
                };
                let scale = ew + 1;
                // Each entity type gets a distinct hue spaced 99° apart around the color wheel.
                let color = hsl_to_rgb((99 * ekind.as_usize()) as f64, 0.99, 0.50);
                // Iterate `l = 55` down to `1` (skipping `l = 0` to match JS `l--` falsy exit).
                for l in (1..56).rev() {
                    let u_bit = l & 7; // Row index (0..8)
                    let v_col = l >> 3; // Column index (0..7)
                    let sprite_idx = 7 * ekind.as_usize() + v_col;
                    if sprite_idx < SPRITE_DATA.len()
                        && (((69 ^ SPRITE_DATA[sprite_idx]) >> u_bit) & 1) != 0
                    {
                        // Center sprite around `(ex, ey)`, flip horizontally with `direction`,
                        // and apply `scale` for warp-in / explosion spread.
                        let px = to_i32(
                            ex - self.camera_x
                                + 127.0
                                + ((v_col as i32 - 3) * self.direction * scale) as f64,
                        ) & self.world_mask;
                        let py = to_i32(ey + (scale * (u_bit as i32 - 3)) as f64);
                        if px < 255 {
                            self.plot_pixel(px, py, color);
                        }
                    }
                }

                if remove_entity {
                    self.entities.remove(idx);
                }
            }

            // --- 5. Entity Spawning ---
            // At the end of each frame (`d == 0`):
            // - Tick 0 spawns the Player (`L = 1`, `y = 70`).
            // - Ticks 1..=10 spawn 10 Humans (`L = 2`, `y = 140`) spaced every 99px.
            // - Every 99 ticks after that spawns a new Lander (`L = 4`, `y = 40`).
            if d == 0 {
                if self.ticks < 11 {
                    spawn_code = if self.ticks == 0 { 1 } else { 2 };
                } else if self.ticks % 99 == 0 {
                    spawn_code = 4;
                }
                self.spawn_y = ((70 * (spawn_code as i32)) % 240) as f64;
                self.spawn_x = (99 * self.ticks) as f64;
            }

            if spawn_code != 0 {
                let sx = self.spawn_x;
                let sy = self.spawn_y;
                self.spawn_entity(spawn_code, sx, sy);
                spawn_code = 0;
            }
        }

        self.ticks = self.ticks.wrapping_add(1);
    }
}

/// Converts a floating-point number to a wrapping 32-bit signed integer,
/// matching JavaScript's `ToInt32` conversion used by bitwise operators (`&`, `|`, `<<`).
///
/// Casting `f64 -> i64 -> i32` truncates toward zero and wraps modulo `2^32`
/// instead of saturating at `i32::MAX` / `i32::MIN`.
#[inline]
pub fn to_i32(v: f64) -> i32 {
    (v as i64) as i32
}

/// Converts a CSS HSL color (`h` in degrees, `s` and `l` in `0.0..=1.0`) to an 8-bit `(R, G, B)` tuple.
///
/// Used so our Rust software renderer produces the exact same vibrant neon palette
/// as Canvas2D's `hsl(C, 99%, 50%)` strings in the original JavaScript version.
pub fn hsl_to_rgb(h: f64, s: f64, l: f64) -> (u8, u8, u8) {
    // Normalize any positive or negative hue angle into `[0.0, 360.0)`.
    let hue = ((h % 360.0) + 360.0) % 360.0;
    // Standard HSL-to-RGB chroma (`c`), secondary component (`x`), and lightness offset (`m`).
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let x = c * (1.0 - ((hue / 60.0) % 2.0 - 1.0).abs());
    let m = l - c / 2.0;

    // Pick RGB sector based on which 60° slice of the color wheel `hue` falls in.
    let (r1, g1, b1) = if hue < 60.0 {
        (c, x, 0.0)
    } else if hue < 120.0 {
        (x, c, 0.0)
    } else if hue < 180.0 {
        (0.0, c, x)
    } else if hue < 240.0 {
        (0.0, x, c)
    } else if hue < 300.0 {
        (x, 0.0, c)
    } else {
        (c, 0.0, x)
    };

    // Scale each channel from `0.0..=1.0` to `0..=255`.
    (
        ((r1 + m) * 255.0).round().clamp(0.0, 255.0) as u8,
        ((g1 + m) * 255.0).round().clamp(0.0, 255.0) as u8,
        ((b1 + m) * 255.0).round().clamp(0.0, 255.0) as u8,
    )
}

// `#[cfg(test)]` tells Cargo to compile this entire module ONLY when running `cargo test`,
// so unit test code adds zero bytes to the release WebAssembly binary.
#[cfg(test)]
mod tests {
    // Import everything from the parent `game` module (`super`).
    use super::*;

    /// Verifies that the first 11 ticks spawn 1 Player at index 0 followed by 10 Humans.
    #[test]
    fn initial_spawns_player_and_ten_humans() {
        let mut game = Game::new();
        for _ in 0..11 {
            game.tick();
        }
        assert_eq!(game.entities.len(), 11);
        assert_eq!(game.entities[0].kind, EntityKind::Player);
        assert_eq!(game.humans_alive(), 10);
        assert_eq!(game.enemies_alive(), 0);
        assert!(!game.game_over);
    }

    /// Verifies that the first Lander spawns at tick 99 and begins in its warp-in state (`w > 0`).
    #[test]
    fn lander_spawns_every_99_ticks() {
        let mut game = Game::new();
        for _ in 0..100 {
            game.tick();
        }
        assert_eq!(game.enemies_alive(), 1);
        let lander = game
            .entities
            .iter()
            .find(|e| e.kind == EntityKind::Lander)
            .expect("lander should have spawned at tick 99");
        assert!(lander.warp_explosion > 0, "new lander should warp in");
    }

    /// Verifies horizontal thrust, facing direction flips, and vertical climb/dive controls.
    #[test]
    fn player_movement_and_direction() {
        let mut game = Game::new();
        game.tick(); // Spawn player

        // Thrust right and climb
        game.set_input(InputState {
            right: true,
            up: true,
            ..Default::default()
        });
        let start_x = game.entities[0].x;
        let start_y = game.entities[0].y;
        for _ in 0..10 {
            game.tick();
        }
        assert_eq!(game.direction, 1);
        assert!(game.entities[0].x > start_x);
        assert!(game.entities[0].y < start_y);

        // Thrust left and dive
        game.set_input(InputState {
            left: true,
            down: true,
            ..Default::default()
        });
        for _ in 0..20 {
            game.tick();
        }
        assert_eq!(game.direction, -1);
    }

    /// Verifies that firing the laser destroys an aligned Lander in front of the ship and awards 150 points.
    #[test]
    fn player_laser_destroys_lander_in_front() {
        let mut game = Game::new();
        game.tick(); // Spawn player at (0, 70)
        // Place an active lander 40px in front of player at same Y
        game.entities.push(Entity {
            id: 999,
            kind: EntityKind::Lander,
            speed: EntityKind::Lander.speed(),
            warp_explosion: 0,
            target_id: None,
            vx: 0.0,
            vy: 0.0,
            timer: 50,
            x: 40.0,
            y: 70.0,
        });

        game.set_input(InputState {
            fire: true,
            ..Default::default()
        });
        // Tick 1 triggers firing (J = true at d=0), Tick 2 hits the lander
        game.tick();
        game.tick();

        let lander = game
            .entities
            .iter()
            .find(|e| e.id == 999)
            .expect("exploding lander still exists briefly");
        assert!(lander.warp_explosion < 0, "lander should be exploding");
        assert_eq!(game.score, 150);
    }

    /// Verifies that an abducted Human reaching the top of the screen (`y < 1`) spawns a Mutant.
    #[test]
    fn human_abduction_spawns_mutant_at_top() {
        let mut game = Game::new();
        game.tick(); // Spawn player
        // Place a grabbed human near top of screen
        game.entities.push(Entity {
            id: 500,
            kind: EntityKind::Human,
            speed: EntityKind::Human.speed(),
            warp_explosion: 0,
            target_id: Some(501),
            vx: 0.0,
            vy: -1.0,
            timer: 0,
            x: 200.0,
            y: 0.5,
        });

        game.tick();

        let has_mutant = game.entities.iter().any(|e| e.kind == EntityKind::Mutant);
        assert!(has_mutant, "abducted human reaching top should spawn a mutant");
    }

    /// Verifies that colliding with an enemy explodes the Player and triggers Game Over (`world_mask == 0`).
    #[test]
    fn enemy_collision_kills_player_and_ends_game() {
        let mut game = Game::new();
        game.tick(); // Spawn player at (0, 70)
        game.entities.push(Entity {
            id: 777,
            kind: EntityKind::Lander,
            speed: 0.0,
            warp_explosion: 0,
            target_id: None,
            vx: 0.0,
            vy: 0.0,
            timer: 99,
            x: 1.0,
            y: 70.0,
        });

        // Run until player finishes exploding (> 26 ticks)
        for _ in 0..35 {
            game.tick();
        }
        assert!(game.game_over);
        assert_eq!(game.world_mask, 0);
    }

    /// Verifies that the software renderer draws visible non-black scenery and sprites into the framebuffer.
    #[test]
    fn framebuffer_renders_non_black_pixels() {
        let mut game = Game::new();
        for _ in 0..15 {
            game.tick();
        }
        let non_black = game
            .framebuffer
            .chunks_exact(4)
            .filter(|px| px[0] > 0 || px[1] > 0 || px[2] > 0)
            .count();
        assert!(
            non_black > 200,
            "expected mountains, stars, and sprites to render, got {non_black} lit pixels"
        );
    }

    /// Verifies `hsl_to_rgb` against primary red, green, blue, and negative hue wrap-around.
    #[test]
    fn hsl_to_rgb_produces_expected_primaries() {
        assert_eq!(hsl_to_rgb(0.0, 1.0, 0.5), (255, 0, 0));
        assert_eq!(hsl_to_rgb(120.0, 1.0, 0.5), (0, 255, 0));
        assert_eq!(hsl_to_rgb(240.0, 1.0, 0.5), (0, 0, 255));
        assert_eq!(hsl_to_rgb(-120.0, 1.0, 0.5), (0, 0, 255));
    }

    /// Verifies that `Game::reset()` clears score, ticks, entities, and restores `WORLD_MASK`.
    #[test]
    fn reset_restores_clean_state() {
        let mut game = Game::new();
        for _ in 0..50 {
            game.tick();
        }
        game.score = 999;
        game.game_over = true;
        game.reset();

        assert_eq!(game.ticks, 0);
        assert_eq!(game.score, 0);
        assert!(!game.game_over);
        assert_eq!(game.world_mask, WORLD_MASK);
        assert!(game.entities.is_empty());
    }

    /// Verifies that destroying an abducting Lander unlinks its Human so the Human falls back to Earth.
    #[test]
    fn rescuing_abducted_human_unlinks_and_falls_to_ground() {
        let mut game = Game::new();
        game.tick(); // Spawn player

        // Add a linked human + lander pair mid-abduction
        let human_id = 100;
        let lander_id = 101;
        game.entities.push(Entity {
            id: human_id,
            kind: EntityKind::Human,
            speed: EntityKind::Human.speed(),
            warp_explosion: 0,
            target_id: Some(lander_id),
            vx: 0.0,
            vy: -1.0,
            timer: 0,
            x: 300.0,
            y: 80.0,
        });
        game.entities.push(Entity {
            id: lander_id,
            kind: EntityKind::Lander,
            speed: EntityKind::Lander.speed(),
            warp_explosion: -1, // Lander was just shot!
            target_id: Some(human_id),
            vx: 0.0,
            vy: 0.0,
            timer: 50,
            x: 300.0,
            y: 72.0,
        });

        game.tick();

        let human = game
            .entities
            .iter()
            .find(|e| e.id == human_id)
            .expect("human should still be alive");
        assert_eq!(human.target_id, None, "human should be unlinked from dead lander");
        assert_eq!(human.vy, 1.0, "rescued human should fall back toward earth");
    }

    /// Verifies that laser hit detection wraps cleanly across the 1024-pixel world seam.
    #[test]
    fn world_wraps_cleanly_across_1024_boundary() {
        let mut game = Game::new();
        game.tick(); // Spawn player at x=0
        game.entities[0].x = 1020.0;
        // Place lander just across the wrap boundary at x=10 (14px ahead of player)
        game.entities.push(Entity {
            id: 888,
            kind: EntityKind::Lander,
            speed: 0.0,
            warp_explosion: 0,
            target_id: None,
            vx: 0.0,
            vy: 0.0,
            timer: 99,
            x: 10.0,
            y: 70.0,
        });

        game.set_input(InputState {
            fire: true,
            ..Default::default()
        });
        game.tick();
        game.tick();

        let lander = game.entities.iter().find(|e| e.id == 888).unwrap();
        assert!(
            lander.warp_explosion < 0,
            "laser should hit lander across 1024 world wrap boundary"
        );
    }

    /// Verifies the exact 35-byte JS1K `/final/` sprite table and blank outer columns on Lander & Mutant.
    #[test]
    fn sprite_data_is_35_bytes_and_lander_mutant_have_blank_outer_columns() {
        assert_eq!(SPRITE_DATA.len(), 35);
        assert_eq!(&SPRITE_DATA[0..7], b"[y}}U]U");
        assert_eq!(&SPRITE_DATA[7..14], b"MA8Z8AM");
        assert_eq!(&SPRITE_DATA[14..21], b"EMYYMEE");
        assert_eq!(&SPRITE_DATA[21..28], b"E{V:V{E");
        assert_eq!(&SPRITE_DATA[28..35], b"E{V:V{E");
        // Outer columns of Lander & Mutant must decode to 0 (blank)
        assert_eq!(69 ^ SPRITE_DATA[21], 0);
        assert_eq!(69 ^ SPRITE_DATA[27], 0);
        assert_eq!(69 ^ SPRITE_DATA[28], 0);
        assert_eq!(69 ^ SPRITE_DATA[34], 0);
    }

    /// Verifies that screen dimensions match the original `510 × 320` canvas.
    #[test]
    fn screen_dimensions_match_js1k_510x320() {
        assert_eq!(LOGICAL_WIDTH, 255);
        assert_eq!(LOGICAL_HEIGHT, 160);
        assert_eq!(SCREEN_WIDTH, 510);
        assert_eq!(SCREEN_HEIGHT, 320);
        assert_eq!(FRAMEBUFFER_SIZE, 510 * 320 * 4);
    }

    /// Verifies that ticks after Game Over leave the final rendered frame frozen in place.
    #[test]
    fn game_over_freezes_last_frame_instead_of_clearing_black() {
        let mut game = Game::new();
        game.tick(); // Spawn player
        game.entities.push(Entity {
            id: 777,
            kind: EntityKind::Lander,
            speed: 0.0,
            warp_explosion: 0,
            target_id: None,
            vx: 0.0,
            vy: 0.0,
            timer: 99,
            x: 1.0,
            y: 70.0,
        });

        while !game.game_over {
            game.tick();
        }
        let frozen_fb = game.framebuffer.clone();
        let non_black_before = frozen_fb
            .chunks_exact(4)
            .filter(|px| px[0] > 0 || px[1] > 0 || px[2] > 0)
            .count();
        assert!(
            non_black_before > 100,
            "final frame before freeze should have visible scenery/sprites"
        );

        // Additional ticks after game over must NOT wipe the framebuffer black
        for _ in 0..5 {
            game.tick();
        }
        assert_eq!(game.framebuffer, frozen_fb);
    }

    /// Verifies that an enemy remaining on top of the player decrements `E[0].w` every tick (matching JS).
    #[test]
    fn repeated_enemy_collision_accelerates_player_explosion() {
        let mut game = Game::new();
        game.tick(); // Spawn player at (0, 70)
        game.entities.push(Entity {
            id: 778,
            kind: EntityKind::Lander,
            speed: 0.0,
            warp_explosion: 0,
            target_id: None,
            vx: 0.0,
            vy: 0.0,
            timer: 99,
            x: 0.0,
            y: 70.0,
        });

        // Tick 1: Lander decrements E[0].w from 0 -> -1, then Player (d=0) sees w=-1 and decrements -> -2.
        game.tick();
        assert_eq!(game.entities[0].warp_explosion, -2);
        // Tick 2: Lander decrements -> -3, Player decrements -> -4.
        game.tick();
        assert_eq!(game.entities[0].warp_explosion, -4);
    }

    /// Verifies that floating-point subtraction happens before `to_i32` truncation, matching JS `ToInt32`.
    #[test]
    fn float_before_to_i32_truncation_matches_js_semantics() {
        // JS `(100 - 10.6) & 1023 === 89`, whereas `(100 - (10.6|0)) & 1023 === 90`.
        assert_eq!(to_i32(100.0_f64 - 10.6_f64) & WORLD_MASK, 89);
        assert_eq!(to_i32(10.2_f64 - 9.8_f64) & WORLD_MASK, 0);
        // Negative crossing 0 for Y: `to_i32(0.5 - 3.0) == -2`, whereas `to_i32(0.5) - 3 == -3`.
        assert_eq!(to_i32(0.5_f64 + (-3.0_f64)), -2);
    }

    /// Runs a 150-tick scripted maneuver to verify deterministic multi-tick simulation stability.
    #[test]
    fn deterministic_multi_tick_parity_scripted_run() {
        let mut game = Game::new();
        for t in 0..150 {
            let input = match t {
                0..=29 => InputState {
                    right: true,
                    up: true,
                    ..Default::default()
                },
                30..=59 => InputState {
                    right: true,
                    fire: true,
                    ..Default::default()
                },
                60..=99 => InputState {
                    left: true,
                    down: true,
                    ..Default::default()
                },
                _ => InputState {
                    fire: t % 20 < 5,
                    ..Default::default()
                },
            };
            game.set_input(input);
            game.tick();
        }
        assert_eq!(game.ticks, 150);
        assert!(!game.game_over);
        assert_eq!(game.direction, -1);
        assert!(game.enemies_alive() >= 1);
    }

    /// Regression test ensuring the player cannot hit itself with its own laser at maximum horizontal speed.
    #[test]
    fn player_does_not_shoot_self_at_high_speed() {
        let mut game = Game::new();
        // Accelerate right to max speed (vx = 25, step = 6.25 >= 5.0) while firing
        for _ in 0..60 {
            game.set_input(InputState {
                right: true,
                fire: true,
                ..Default::default()
            });
            game.tick();
        }
        assert!(!game.game_over);
        assert_eq!(game.entities[0].kind, EntityKind::Player);
        assert_eq!(
            game.entities[0].warp_explosion, 0,
            "player must not hit itself with its own laser at high velocity"
        );
    }
}
