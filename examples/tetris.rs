//! Tetris for the Disobey 2026 badge — championship rules.
//!
//! Implements the modern Tetris guideline:
//! - SRS (Super Rotation System) with wall kicks
//! - 7-bag randomizer
//! - Ghost piece
//! - Lock delay with move reset
//! - T-spin detection (single, double, triple)
//! - Back-to-back bonus for Tetris / T-spins
//! - Combo system
//! - Increasing levels and gravity
//! - Hold piece (Select button)
//!
//! Controls:
//! - Left/Right: move piece
//! - Down: soft drop
//! - Up: hard drop
//! - A: rotate clockwise
//! - B: rotate counter-clockwise
//! - Select: hold piece
//! - Start: pause / restart after game over

#![no_std]
#![no_main]

use core::sync::atomic::{AtomicBool, AtomicU8, Ordering};

use defmt::info;
#[allow(clippy::wildcard_imports)]
use disobey2026badge::*;
use embassy_executor::Spawner;
use embassy_sync::channel::Channel;
use embassy_time::{Duration, Timer};
use embedded_graphics::{
    mono_font::{MonoTextStyle, ascii::FONT_6X10},
    pixelcolor::Rgb565,
    prelude::*,
    primitives::{PrimitiveStyle, Rectangle},
    text::Text,
};
use esp_backtrace as _;
use esp_hal::timer::timg::TimerGroup;
use esp_println as _;
use palette::Srgb;

extern crate alloc;

esp_bootloader_esp_idf::esp_app_desc!();

// ── Display / board geometry ────────────────────────────────────────────────
const SCREEN_W: i32 = 320;
const SCREEN_H: i32 = 170;

const CELL: i32 = 8; // pixel size of one tetris cell
const BOARD_W: usize = 10;
const BOARD_H: usize = 20;
const BOARD_PX_W: i32 = BOARD_W as i32 * CELL;
const BOARD_PX_H: i32 = BOARD_H as i32 * CELL;
const BOARD_X: i32 = (SCREEN_W - BOARD_PX_W) / 2; // centered
const BOARD_Y: i32 = (SCREEN_H - BOARD_PX_H) / 2;

// HUD positions
const HOLD_X: i32 = BOARD_X - 42;
const HOLD_Y: i32 = BOARD_Y + 14;
const NEXT_X: i32 = BOARD_X + BOARD_PX_W + 10;
const NEXT_Y: i32 = BOARD_Y + 14;
const SCORE_X: i32 = BOARD_X + BOARD_PX_W + 10;
const SCORE_Y: i32 = BOARD_Y + 80;
const LEVEL_X: i32 = BOARD_X + BOARD_PX_W + 10;
const LEVEL_Y: i32 = BOARD_Y + 110;

const TICK_MS: u64 = 16; // ~60fps frame tick
const DAS_DELAY: u8 = 10; // frames before auto-repeat starts
const ARR_RATE: u8 = 2; // frames between auto-repeat moves
const LOCK_DELAY_FRAMES: u8 = 30; // 0.5s at 60fps
const MAX_LOCK_RESETS: u8 = 15;

// ── Input atomics ───────────────────────────────────────────────────────────
static INPUT_LEFT: AtomicBool = AtomicBool::new(false);
static INPUT_RIGHT: AtomicBool = AtomicBool::new(false);
static INPUT_DOWN: AtomicBool = AtomicBool::new(false);
static INPUT_UP: AtomicBool = AtomicBool::new(false);
static INPUT_A: AtomicBool = AtomicBool::new(false);
static INPUT_B: AtomicBool = AtomicBool::new(false);
static INPUT_SELECT: AtomicBool = AtomicBool::new(false);
static INPUT_START: AtomicBool = AtomicBool::new(false);

// Edge detection: set to 1 by input task, consumed (set to 0) by game
static EDGE_UP: AtomicU8 = AtomicU8::new(0);
static EDGE_A: AtomicU8 = AtomicU8::new(0);
static EDGE_B: AtomicU8 = AtomicU8::new(0);
static EDGE_SELECT: AtomicU8 = AtomicU8::new(0);
static EDGE_START: AtomicU8 = AtomicU8::new(0);

// ── LED events ──────────────────────────────────────────────────────────────
#[derive(Clone, Copy)]
enum LedEvent {
    LineClear(u8),
    TSpin,
    GameOver,
    LevelUp,
}

static LED_CHANNEL: Channel<
    embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex,
    LedEvent,
    4,
> = Channel::new();

// ── Vibration events ────────────────────────────────────────────────────────
#[derive(Clone, Copy)]
enum VibraEvent {
    Drop,
    LineClear,
    Tetris,
}

static VIBRA_CHANNEL: Channel<
    embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex,
    VibraEvent,
    4,
> = Channel::new();

// ── Piece definitions (SRS) ─────────────────────────────────────────────────
// Each piece has 4 rotation states, each state is 4 (x,y) offsets from pivot.
// Coordinates: +x right, +y down.

#[derive(Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
enum PieceKind {
    I = 0,
    O = 1,
    T = 2,
    S = 3,
    Z = 4,
    J = 5,
    L = 6,
}

impl PieceKind {
    fn color(self) -> Rgb565 {
        match self {
            PieceKind::I => Rgb565::CYAN,
            PieceKind::O => Rgb565::YELLOW,
            PieceKind::T => Rgb565::CSS_PURPLE,
            PieceKind::S => Rgb565::GREEN,
            PieceKind::Z => Rgb565::RED,
            PieceKind::J => Rgb565::BLUE,
            PieceKind::L => Rgb565::CSS_ORANGE,
        }
    }

    fn from_index(i: usize) -> Self {
        match i {
            0 => PieceKind::I,
            1 => PieceKind::O,
            2 => PieceKind::T,
            3 => PieceKind::S,
            4 => PieceKind::Z,
            5 => PieceKind::J,
            _ => PieceKind::L,
        }
    }

    /// 4 rotation states × 4 cells, each cell is (dx, dy) from piece origin.
    fn cells(self) -> &'static [[(i8, i8); 4]; 4] {
        match self {
            PieceKind::I => &[
                [(-1, 0), (0, 0), (1, 0), (2, 0)],
                [(0, -1), (0, 0), (0, 1), (0, 2)],
                [(-1, 1), (0, 1), (1, 1), (2, 1)],
                [(1, -1), (1, 0), (1, 1), (1, 2)],
            ],
            PieceKind::O => &[
                [(0, 0), (1, 0), (0, 1), (1, 1)],
                [(0, 0), (1, 0), (0, 1), (1, 1)],
                [(0, 0), (1, 0), (0, 1), (1, 1)],
                [(0, 0), (1, 0), (0, 1), (1, 1)],
            ],
            PieceKind::T => &[
                [(-1, 0), (0, 0), (1, 0), (0, -1)],
                [(0, -1), (0, 0), (0, 1), (1, 0)],
                [(-1, 0), (0, 0), (1, 0), (0, 1)],
                [(0, -1), (0, 0), (0, 1), (-1, 0)],
            ],
            PieceKind::S => &[
                [(-1, 0), (0, 0), (0, -1), (1, -1)],
                [(0, -1), (0, 0), (1, 0), (1, 1)],
                [(-1, 1), (0, 1), (0, 0), (1, 0)],
                [(-1, -1), (-1, 0), (0, 0), (0, 1)],
            ],
            PieceKind::Z => &[
                [(-1, -1), (0, -1), (0, 0), (1, 0)],
                [(1, -1), (1, 0), (0, 0), (0, 1)],
                [(-1, 0), (0, 0), (0, 1), (1, 1)],
                [(0, -1), (0, 0), (-1, 0), (-1, 1)],
            ],
            PieceKind::J => &[
                [(-1, -1), (-1, 0), (0, 0), (1, 0)],
                [(0, -1), (0, 0), (0, 1), (1, -1)],
                [(-1, 0), (0, 0), (1, 0), (1, 1)],
                [(-1, 1), (0, -1), (0, 0), (0, 1)],
            ],
            PieceKind::L => &[
                [(-1, 0), (0, 0), (1, 0), (1, -1)],
                [(0, -1), (0, 0), (0, 1), (1, 1)],
                [(-1, 1), (-1, 0), (0, 0), (1, 0)],
                [(-1, -1), (0, -1), (0, 0), (0, 1)],
            ],
        }
    }
}

// ── SRS Wall Kick data ──────────────────────────────────────────────────────
// For each rotation transition, 5 kick offsets to try (including (0,0)).
// JLSTZ kicks and I kicks are different per the guideline.

/// JLSTZ wall kick offsets: from_rot → 4 transitions (CW), each with 5 tests.
const KICK_JLSTZ: [[(i8, i8); 5]; 8] = [
    // 0→1
    [(0, 0), (-1, 0), (-1, -1), (0, 2), (-1, 2)],
    // 1→2
    [(0, 0), (1, 0), (1, 1), (0, -2), (1, -2)],
    // 2→3
    [(0, 0), (1, 0), (1, -1), (0, 2), (1, 2)],
    // 3→0
    [(0, 0), (-1, 0), (-1, 1), (0, -2), (-1, -2)],
    // 0→3 (CCW)
    [(0, 0), (1, 0), (1, -1), (0, 2), (1, 2)],
    // 3→2
    [(0, 0), (-1, 0), (-1, 1), (0, -2), (-1, -2)],
    // 2→1
    [(0, 0), (-1, 0), (-1, -1), (0, 2), (-1, 2)],
    // 1→0
    [(0, 0), (1, 0), (1, 1), (0, -2), (1, -2)],
];

const KICK_I: [[(i8, i8); 5]; 8] = [
    // 0→1
    [(0, 0), (-2, 0), (1, 0), (-2, 1), (1, -2)],
    // 1→2
    [(0, 0), (-1, 0), (2, 0), (-1, -2), (2, 1)],
    // 2→3
    [(0, 0), (2, 0), (-1, 0), (2, -1), (-1, 2)],
    // 3→0
    [(0, 0), (1, 0), (-2, 0), (1, 2), (-2, -1)],
    // 0→3 (CCW)
    [(0, 0), (-1, 0), (2, 0), (-1, -2), (2, 1)],
    // 3→2
    [(0, 0), (-2, 0), (1, 0), (-2, 1), (1, -2)],
    // 2→1
    [(0, 0), (1, 0), (-2, 0), (1, 2), (-2, -1)],
    // 1→0
    [(0, 0), (2, 0), (-1, 0), (2, -1), (-1, 2)],
];

fn kick_index_cw(from: u8) -> usize {
    from as usize // 0→1=0, 1→2=1, 2→3=2, 3→0=3
}

fn kick_index_ccw(from: u8) -> usize {
    4 + ((4 - from) % 4) as usize // 0→3=4, 3→2=5, 2→1=6, 1→0=7
}

// ── Simple RNG (xorshift) ───────────────────────────────────────────────────
struct Rng(u32);
impl Rng {
    const fn new(seed: u32) -> Self {
        Self(seed)
    }
    fn next(&mut self) -> u32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 17;
        self.0 ^= self.0 << 5;
        self.0
    }
    fn range(&mut self, max: u32) -> u32 {
        self.next() % max
    }
}

// ── 7-bag randomizer ────────────────────────────────────────────────────────
struct Bag {
    pieces: [u8; 7],
    index: usize,
    rng: Rng,
}

impl Bag {
    fn new(seed: u32) -> Self {
        let mut b = Self {
            pieces: [0, 1, 2, 3, 4, 5, 6],
            index: 7,
            rng: Rng::new(seed),
        };
        b.shuffle();
        b.index = 0;
        b
    }

    fn shuffle(&mut self) {
        for i in (1..7).rev() {
            let j = self.rng.range(i as u32 + 1) as usize;
            self.pieces.swap(i, j);
        }
    }

    fn next(&mut self) -> PieceKind {
        if self.index >= 7 {
            self.shuffle();
            self.index = 0;
        }
        let kind = PieceKind::from_index(self.pieces[self.index] as usize);
        self.index += 1;
        kind
    }

    fn peek(&self) -> PieceKind {
        if self.index < 7 {
            PieceKind::from_index(self.pieces[self.index] as usize)
        } else {
            // Would need to peek into next bag — just show first of current
            PieceKind::from_index(self.pieces[0] as usize)
        }
    }
}

// ── Active piece ────────────────────────────────────────────────────────────
#[derive(Clone, Copy)]
struct ActivePiece {
    kind: PieceKind,
    x: i8,
    y: i8,
    rot: u8, // 0..3
}

impl ActivePiece {
    fn spawn(kind: PieceKind) -> Self {
        Self {
            kind,
            x: (BOARD_W as i8) / 2 - 1,
            y: 0,
            rot: 0,
        }
    }

    fn cells(&self) -> [(i8, i8); 4] {
        let template = self.kind.cells()[self.rot as usize];
        let mut out = [(0i8, 0i8); 4];
        for i in 0..4 {
            out[i] = (self.x + template[i].0, self.y + template[i].1);
        }
        out
    }

    fn moved(&self, dx: i8, dy: i8) -> Self {
        Self {
            x: self.x + dx,
            y: self.y + dy,
            ..*self
        }
    }

    fn rotated_cw(&self) -> Self {
        Self {
            rot: (self.rot + 1) % 4,
            ..*self
        }
    }

    fn rotated_ccw(&self) -> Self {
        Self {
            rot: (self.rot + 3) % 4,
            ..*self
        }
    }
}

// ── Board ───────────────────────────────────────────────────────────────────
// Each cell: 0 = empty, 1..7 = piece kind + 1
type Board = [[u8; BOARD_W]; BOARD_H];

fn empty_board() -> Board {
    [[0u8; BOARD_W]; BOARD_H]
}

fn fits(board: &Board, piece: &ActivePiece) -> bool {
    for (cx, cy) in piece.cells() {
        if cx < 0 || cx >= BOARD_W as i8 || cy >= BOARD_H as i8 {
            return false;
        }
        if cy < 0 {
            continue; // above board is ok
        }
        if board[cy as usize][cx as usize] != 0 {
            return false;
        }
    }
    true
}

fn lock_piece(board: &mut Board, piece: &ActivePiece) {
    let color_id = piece.kind as u8 + 1;
    for (cx, cy) in piece.cells() {
        if cy >= 0 && (cy as usize) < BOARD_H && cx >= 0 && (cx as usize) < BOARD_W {
            board[cy as usize][cx as usize] = color_id;
        }
    }
}

/// Returns number of lines cleared and which rows were cleared.
fn clear_lines(board: &mut Board) -> (u8, [bool; BOARD_H]) {
    let mut cleared = [false; BOARD_H];
    let mut count = 0u8;
    for y in 0..BOARD_H {
        if board[y].iter().all(|&c| c != 0) {
            cleared[y] = true;
            count += 1;
        }
    }
    if count > 0 {
        let mut write = BOARD_H - 1;
        for read in (0..BOARD_H).rev() {
            if !cleared[read] {
                board[write] = board[read];
                if write > 0 {
                    write -= 1;
                }
            }
        }
        // Fill top rows with empty
        for y in 0..count as usize {
            board[y] = [0u8; BOARD_W];
        }
    }
    (count, cleared)
}

/// Ghost piece: drop piece as far as it goes.
fn ghost_y(board: &Board, piece: &ActivePiece) -> i8 {
    let mut test = *piece;
    while fits(board, &test.moved(0, 1)) {
        test.y += 1;
    }
    test.y
}

/// T-spin detection: after locking a T piece, check if 3 of 4 corners are filled.
fn is_t_spin(board: &Board, piece: &ActivePiece, last_was_rotation: bool) -> bool {
    if piece.kind != PieceKind::T || !last_was_rotation {
        return false;
    }
    let corners = [
        (piece.x - 1, piece.y - 1),
        (piece.x + 1, piece.y - 1),
        (piece.x - 1, piece.y + 1),
        (piece.x + 1, piece.y + 1),
    ];
    let mut filled = 0u8;
    for (cx, cy) in corners {
        if cx < 0 || cx >= BOARD_W as i8 || cy < 0 || cy >= BOARD_H as i8 {
            filled += 1; // walls/floor count as filled
        } else if board[cy as usize][cx as usize] != 0 {
            filled += 1;
        }
    }
    filled >= 3
}

// ── Scoring (guideline) ─────────────────────────────────────────────────────
fn line_clear_score(lines: u8, t_spin: bool, b2b: bool, combo: u8, level: u8) -> u32 {
    let base: u32 = if t_spin {
        match lines {
            1 => 800,
            2 => 1200,
            3 => 1600,
            _ => 0,
        }
    } else {
        match lines {
            1 => 100,
            2 => 300,
            3 => 500,
            4 => 800, // Tetris
            _ => 0,
        }
    };
    let b2b_mult: u32 = if b2b { 3 } else { 2 };
    let combo_bonus: u32 = 50 * combo as u32 * level as u32;
    (base * b2b_mult / 2) * level as u32 + combo_bonus
}

fn soft_drop_score(cells: u32) -> u32 {
    cells
}

fn hard_drop_score(cells: u32) -> u32 {
    cells * 2
}

/// Gravity: frames per drop at each level (guideline approximation).
fn gravity_frames(level: u8) -> u8 {
    match level {
        1 => 48,
        2 => 43,
        3 => 38,
        4 => 33,
        5 => 28,
        6 => 23,
        7 => 18,
        8 => 13,
        9 => 8,
        10 => 6,
        11..=12 => 5,
        13..=15 => 4,
        16..=18 => 3,
        19..=28 => 2,
        _ => 1,
    }
}

// ── Game state ──────────────────────────────────────────────────────────────
struct Game {
    board: Board,
    piece: ActivePiece,
    bag: Bag,
    hold: Option<PieceKind>,
    hold_used: bool, // can only hold once per piece
    score: u32,
    level: u8,
    lines_total: u32,
    combo: u8,
    back_to_back: bool,
    game_over: bool,
    paused: bool,
    // Gravity / lock delay
    gravity_counter: u8,
    lock_counter: u8,
    lock_resets: u8,
    on_ground: bool,
    last_was_rotation: bool,
    // DAS (delayed auto shift)
    das_left: u8,
    das_right: u8,
    prev_left: bool,
    prev_right: bool,
    prev_down: bool,
}

impl Game {
    fn new() -> Self {
        let mut bag = Bag::new(0xCAFE_BABE);
        let kind = bag.next();
        Self {
            board: empty_board(),
            piece: ActivePiece::spawn(kind),
            bag,
            hold: None,
            hold_used: false,
            score: 0,
            level: 1,
            lines_total: 0,
            combo: 0,
            back_to_back: false,
            game_over: false,
            paused: false,
            gravity_counter: 0,
            lock_counter: 0,
            lock_resets: 0,
            on_ground: false,
            last_was_rotation: false,
            das_left: 0,
            das_right: 0,
            prev_left: false,
            prev_right: false,
            prev_down: false,
        }
    }

    fn spawn_next(&mut self) {
        let kind = self.bag.next();
        self.piece = ActivePiece::spawn(kind);
        self.hold_used = false;
        self.gravity_counter = 0;
        self.lock_counter = 0;
        self.lock_resets = 0;
        self.on_ground = false;
        self.last_was_rotation = false;
        if !fits(&self.board, &self.piece) {
            self.game_over = true;
        }
    }

    fn try_move(&mut self, dx: i8, dy: i8) -> bool {
        let moved = self.piece.moved(dx, dy);
        if fits(&self.board, &moved) {
            self.piece = moved;
            self.last_was_rotation = false;
            self.reset_lock_if_on_ground();
            return true;
        }
        false
    }

    fn try_rotate_cw(&mut self) -> bool {
        self.try_rotate(true)
    }

    fn try_rotate_ccw(&mut self) -> bool {
        self.try_rotate(false)
    }

    fn try_rotate(&mut self, clockwise: bool) -> bool {
        let rotated = if clockwise {
            self.piece.rotated_cw()
        } else {
            self.piece.rotated_ccw()
        };

        let kick_idx = if clockwise {
            kick_index_cw(self.piece.rot)
        } else {
            kick_index_ccw(self.piece.rot)
        };

        let kicks = if self.piece.kind == PieceKind::I {
            &KICK_I[kick_idx]
        } else {
            &KICK_JLSTZ[kick_idx]
        };

        for &(kx, ky) in kicks {
            let test = ActivePiece {
                x: rotated.x + kx,
                y: rotated.y + ky,
                ..rotated
            };
            if fits(&self.board, &test) {
                self.piece = test;
                self.last_was_rotation = true;
                self.reset_lock_if_on_ground();
                return true;
            }
        }
        false
    }

    fn reset_lock_if_on_ground(&mut self) {
        if self.on_ground && self.lock_resets < MAX_LOCK_RESETS {
            self.lock_counter = 0;
            self.lock_resets += 1;
        }
    }

    fn hard_drop(&mut self) {
        let mut dropped: u32 = 0;
        while fits(&self.board, &self.piece.moved(0, 1)) {
            self.piece.y += 1;
            dropped += 1;
        }
        self.score += hard_drop_score(dropped);
        self.lock_piece_and_clear();
        VIBRA_CHANNEL.try_send(VibraEvent::Drop).ok();
    }

    fn hold_piece(&mut self) {
        if self.hold_used {
            return;
        }
        let current_kind = self.piece.kind;
        if let Some(held) = self.hold {
            self.piece = ActivePiece::spawn(held);
        } else {
            self.spawn_next();
        }
        self.hold = Some(current_kind);
        self.hold_used = true;
        self.gravity_counter = 0;
        self.lock_counter = 0;
        self.lock_resets = 0;
        self.on_ground = false;
    }

    fn lock_piece_and_clear(&mut self) {
        let t_spin = is_t_spin(&self.board, &self.piece, self.last_was_rotation);
        lock_piece(&mut self.board, &self.piece);

        let (lines, _) = clear_lines(&mut self.board);

        if lines > 0 {
            let is_difficult = t_spin || lines == 4;
            let b2b = self.back_to_back && is_difficult;
            self.score += line_clear_score(lines, t_spin, b2b, self.combo, self.level);
            self.combo += 1;
            self.lines_total += lines as u32;

            // Level up every 10 lines
            let new_level = (self.lines_total / 10 + 1).min(30) as u8;
            if new_level > self.level {
                self.level = new_level;
                LED_CHANNEL.try_send(LedEvent::LevelUp).ok();
            }

            if is_difficult {
                self.back_to_back = true;
            } else {
                self.back_to_back = false;
            }

            if t_spin {
                LED_CHANNEL.try_send(LedEvent::TSpin).ok();
            }
            LED_CHANNEL.try_send(LedEvent::LineClear(lines)).ok();

            if lines == 4 {
                VIBRA_CHANNEL.try_send(VibraEvent::Tetris).ok();
            } else {
                VIBRA_CHANNEL.try_send(VibraEvent::LineClear).ok();
            }
        } else {
            self.combo = 0;
        }

        self.spawn_next();
    }

    fn tick(&mut self) {
        if self.game_over || self.paused {
            return;
        }

        // Read edge-triggered inputs
        let hard_drop = EDGE_UP.swap(0, Ordering::Relaxed) > 0;
        let rotate_cw = EDGE_A.swap(0, Ordering::Relaxed) > 0;
        let rotate_ccw = EDGE_B.swap(0, Ordering::Relaxed) > 0;
        let hold = EDGE_SELECT.swap(0, Ordering::Relaxed) > 0;

        // Hold
        if hold {
            self.hold_piece();
            return;
        }

        // Rotation
        if rotate_cw {
            self.try_rotate_cw();
        }
        if rotate_ccw {
            self.try_rotate_ccw();
        }

        // Hard drop
        if hard_drop {
            self.hard_drop();
            return;
        }

        // DAS horizontal movement
        let left = INPUT_LEFT.load(Ordering::Relaxed);
        let right = INPUT_RIGHT.load(Ordering::Relaxed);

        if left && !self.prev_left {
            self.try_move(-1, 0);
            self.das_left = 0;
        } else if left {
            self.das_left += 1;
            if self.das_left >= DAS_DELAY {
                if (self.das_left - DAS_DELAY) % ARR_RATE == 0 {
                    self.try_move(-1, 0);
                }
            }
        } else {
            self.das_left = 0;
        }

        if right && !self.prev_right {
            self.try_move(1, 0);
            self.das_right = 0;
        } else if right {
            self.das_right += 1;
            if self.das_right >= DAS_DELAY {
                if (self.das_right - DAS_DELAY) % ARR_RATE == 0 {
                    self.try_move(1, 0);
                }
            }
        } else {
            self.das_right = 0;
        }

        self.prev_left = left;
        self.prev_right = right;

        // Soft drop
        let down = INPUT_DOWN.load(Ordering::Relaxed);
        if down && !self.prev_down {
            if self.try_move(0, 1) {
                self.score += soft_drop_score(1);
                self.gravity_counter = 0;
            }
        } else if down {
            // Continuous soft drop every frame
            if self.try_move(0, 1) {
                self.score += soft_drop_score(1);
                self.gravity_counter = 0;
            }
        }
        self.prev_down = down;

        // Gravity
        self.gravity_counter += 1;
        if self.gravity_counter >= gravity_frames(self.level) {
            self.gravity_counter = 0;
            if !self.try_move(0, 1) {
                // Can't move down — on ground
                self.on_ground = true;
            }
        }

        // Lock delay
        if !fits(&self.board, &self.piece.moved(0, 1)) {
            self.on_ground = true;
            self.lock_counter += 1;
            if self.lock_counter >= LOCK_DELAY_FRAMES {
                self.lock_piece_and_clear();
            }
        } else {
            self.on_ground = false;
            self.lock_counter = 0;
        }
    }
}

// ── Rendering ───────────────────────────────────────────────────────────────
const BLACK: Rgb565 = Rgb565::BLACK;
const BORDER_COLOR: Rgb565 = Rgb565::new(8, 16, 8);
const GHOST_COLOR: Rgb565 = Rgb565::new(6, 12, 6);
const BG_COLOR: Rgb565 = Rgb565::new(1, 2, 1);

fn color_from_id(id: u8) -> Rgb565 {
    PieceKind::from_index((id.wrapping_sub(1)) as usize).color()
}

fn draw_cell(display: &mut Display, bx: i32, by: i32, color: Rgb565) {
    let px = BOARD_X + bx * CELL;
    let py = BOARD_Y + by * CELL;
    // Outer cell
    Rectangle::new(
        Point::new(px, py),
        Size::new(CELL as u32, CELL as u32),
    )
    .into_styled(PrimitiveStyle::with_fill(color))
    .draw(display)
    .unwrap();
    // Inner highlight (1px border effect)
    if color != BLACK && color != BG_COLOR && color != GHOST_COLOR {
        Rectangle::new(
            Point::new(px + 1, py + 1),
            Size::new((CELL - 2) as u32, (CELL - 2) as u32),
        )
        .into_styled(PrimitiveStyle::with_fill(darken(color)))
        .draw(display)
        .unwrap();
    }
}

fn darken(c: Rgb565) -> Rgb565 {
    let r = c.r() / 2;
    let g = c.g() / 2;
    let b = c.b() / 2;
    Rgb565::new(r, g, b)
}

fn draw_board_border(display: &mut Display) {
    // Left border
    Rectangle::new(
        Point::new(BOARD_X - 2, BOARD_Y - 2),
        Size::new(2, (BOARD_PX_H + 4) as u32),
    )
    .into_styled(PrimitiveStyle::with_fill(BORDER_COLOR))
    .draw(display)
    .unwrap();
    // Right border
    Rectangle::new(
        Point::new(BOARD_X + BOARD_PX_W, BOARD_Y - 2),
        Size::new(2, (BOARD_PX_H + 4) as u32),
    )
    .into_styled(PrimitiveStyle::with_fill(BORDER_COLOR))
    .draw(display)
    .unwrap();
    // Bottom border
    Rectangle::new(
        Point::new(BOARD_X - 2, BOARD_Y + BOARD_PX_H),
        Size::new((BOARD_PX_W + 4) as u32, 2),
    )
    .into_styled(PrimitiveStyle::with_fill(BORDER_COLOR))
    .draw(display)
    .unwrap();
}

fn draw_mini_piece(display: &mut Display, kind: PieceKind, ox: i32, oy: i32) {
    let cells = kind.cells()[0]; // rotation 0
    let s: i32 = 5; // mini cell size
    let color = kind.color();
    for (dx, dy) in cells {
        let px = ox + dx as i32 * s;
        let py = oy + dy as i32 * s;
        Rectangle::new(Point::new(px, py), Size::new(s as u32, s as u32))
            .into_styled(PrimitiveStyle::with_fill(color))
            .draw(display)
            .unwrap();
    }
}

fn clear_mini_area(display: &mut Display, ox: i32, oy: i32) {
    Rectangle::new(Point::new(ox - 10, oy - 10), Size::new(30, 30))
        .into_styled(PrimitiveStyle::with_fill(BLACK))
        .draw(display)
        .unwrap();
}

fn format_u32(mut n: u32, buf: &mut [u8; 16]) -> &str {
    if n == 0 {
        buf[0] = b'0';
        return unsafe { core::str::from_utf8_unchecked(&buf[..1]) };
    }
    let mut i = 0;
    while n > 0 {
        buf[i] = b'0' + (n % 10) as u8;
        n /= 10;
        i += 1;
    }
    buf[..i].reverse();
    unsafe { core::str::from_utf8_unchecked(&buf[..i]) }
}

fn draw_hud(display: &mut Display, game: &Game) {
    let style = MonoTextStyle::new(&FONT_6X10, Rgb565::WHITE);
    let dim = MonoTextStyle::new(&FONT_6X10, Rgb565::new(12, 24, 12));

    // Hold label + piece
    Text::new("HOLD", Point::new(HOLD_X, HOLD_Y - 4), dim)
        .draw(display)
        .unwrap();
    clear_mini_area(display, HOLD_X + 8, HOLD_Y + 12);
    if let Some(kind) = game.hold {
        draw_mini_piece(display, kind, HOLD_X + 8, HOLD_Y + 12);
    }

    // Next label + piece
    Text::new("NEXT", Point::new(NEXT_X, NEXT_Y - 4), dim)
        .draw(display)
        .unwrap();
    clear_mini_area(display, NEXT_X + 8, NEXT_Y + 12);
    draw_mini_piece(display, game.bag.peek(), NEXT_X + 8, NEXT_Y + 12);

    // Score
    Rectangle::new(Point::new(SCORE_X, SCORE_Y - 2), Size::new(60, 22))
        .into_styled(PrimitiveStyle::with_fill(BLACK))
        .draw(display)
        .unwrap();
    Text::new("SCORE", Point::new(SCORE_X, SCORE_Y + 8), dim)
        .draw(display)
        .unwrap();
    let mut buf = [0u8; 16];
    let s = format_u32(game.score, &mut buf);
    Text::new(s, Point::new(SCORE_X, SCORE_Y + 18), style)
        .draw(display)
        .unwrap();

    // Level
    Rectangle::new(Point::new(LEVEL_X, LEVEL_Y - 2), Size::new(60, 22))
        .into_styled(PrimitiveStyle::with_fill(BLACK))
        .draw(display)
        .unwrap();
    Text::new("LEVEL", Point::new(LEVEL_X, LEVEL_Y + 8), dim)
        .draw(display)
        .unwrap();
    let mut buf2 = [0u8; 16];
    let l = format_u32(game.level as u32, &mut buf2);
    Text::new(l, Point::new(LEVEL_X, LEVEL_Y + 18), style)
        .draw(display)
        .unwrap();
}

/// Full board redraw.
fn draw_full_board(display: &mut Display, game: &Game) {
    // Board cells
    for y in 0..BOARD_H {
        for x in 0..BOARD_W {
            let id = game.board[y][x];
            let color = if id == 0 { BG_COLOR } else { color_from_id(id) };
            draw_cell(display, x as i32, y as i32, color);
        }
    }

    // Ghost piece
    let gy = ghost_y(&game.board, &game.piece);
    if gy != game.piece.y {
        let ghost = ActivePiece {
            y: gy,
            ..game.piece
        };
        for (cx, cy) in ghost.cells() {
            if cy >= 0 && (cy as usize) < BOARD_H && cx >= 0 && (cx as usize) < BOARD_W {
                if game.board[cy as usize][cx as usize] == 0 {
                    draw_cell(display, cx as i32, cy as i32, GHOST_COLOR);
                }
            }
        }
    }

    // Active piece
    let color = game.piece.kind.color();
    for (cx, cy) in game.piece.cells() {
        if cy >= 0 && (cy as usize) < BOARD_H && cx >= 0 && (cx as usize) < BOARD_W {
            draw_cell(display, cx as i32, cy as i32, color);
        }
    }
}

/// Incremental draw: erase old piece/ghost, draw new piece/ghost, update changed cells.
fn draw_frame(
    display: &mut Display,
    game: &Game,
    prev_piece: &ActivePiece,
    prev_ghost_y: i8,
    prev_board: &Board,
) {
    // Erase old ghost
    let old_ghost = ActivePiece {
        y: prev_ghost_y,
        ..*prev_piece
    };
    for (cx, cy) in old_ghost.cells() {
        if cy >= 0 && (cy as usize) < BOARD_H && cx >= 0 && (cx as usize) < BOARD_W {
            let id = game.board[cy as usize][cx as usize];
            let color = if id == 0 { BG_COLOR } else { color_from_id(id) };
            draw_cell(display, cx as i32, cy as i32, color);
        }
    }

    // Erase old piece
    for (cx, cy) in prev_piece.cells() {
        if cy >= 0 && (cy as usize) < BOARD_H && cx >= 0 && (cx as usize) < BOARD_W {
            let id = game.board[cy as usize][cx as usize];
            let color = if id == 0 { BG_COLOR } else { color_from_id(id) };
            draw_cell(display, cx as i32, cy as i32, color);
        }
    }

    // Redraw any board cells that changed (line clears, locks)
    for y in 0..BOARD_H {
        for x in 0..BOARD_W {
            if game.board[y][x] != prev_board[y][x] {
                let id = game.board[y][x];
                let color = if id == 0 { BG_COLOR } else { color_from_id(id) };
                draw_cell(display, x as i32, y as i32, color);
            }
        }
    }

    // Draw new ghost
    let gy = ghost_y(&game.board, &game.piece);
    if gy != game.piece.y {
        let ghost = ActivePiece {
            y: gy,
            ..game.piece
        };
        for (cx, cy) in ghost.cells() {
            if cy >= 0 && (cy as usize) < BOARD_H && cx >= 0 && (cx as usize) < BOARD_W {
                if game.board[cy as usize][cx as usize] == 0 {
                    draw_cell(display, cx as i32, cy as i32, GHOST_COLOR);
                }
            }
        }
    }

    // Draw new active piece
    let color = game.piece.kind.color();
    for (cx, cy) in game.piece.cells() {
        if cy >= 0 && (cy as usize) < BOARD_H && cx >= 0 && (cx as usize) < BOARD_W {
            draw_cell(display, cx as i32, cy as i32, color);
        }
    }
}

fn draw_title(display: &mut Display) {
    Rectangle::new(Point::zero(), Size::new(SCREEN_W as u32, SCREEN_H as u32))
        .into_styled(PrimitiveStyle::with_fill(BLACK))
        .draw(display)
        .unwrap();

    let big = MonoTextStyle::new(&FONT_6X10, Rgb565::CYAN);
    let small = MonoTextStyle::new(&FONT_6X10, Rgb565::WHITE);

    Text::new("TETRIS", Point::new(SCREEN_W / 2 - 18, SCREEN_H / 2 - 20), big)
        .draw(display)
        .unwrap();
    Text::new(
        "Championship Edition",
        Point::new(SCREEN_W / 2 - 60, SCREEN_H / 2),
        small,
    )
    .draw(display)
    .unwrap();
    Text::new(
        "Press START",
        Point::new(SCREEN_W / 2 - 33, SCREEN_H / 2 + 20),
        small,
    )
    .draw(display)
    .unwrap();
}

fn draw_game_over(display: &mut Display, score: u32, level: u8) {
    // Darken overlay on board area
    Rectangle::new(
        Point::new(BOARD_X, BOARD_Y),
        Size::new(BOARD_PX_W as u32, BOARD_PX_H as u32),
    )
    .into_styled(PrimitiveStyle::with_fill(Rgb565::new(2, 0, 0)))
    .draw(display)
    .unwrap();

    let style = MonoTextStyle::new(&FONT_6X10, Rgb565::RED);
    let white = MonoTextStyle::new(&FONT_6X10, Rgb565::WHITE);

    Text::new("GAME", Point::new(BOARD_X + 20, BOARD_Y + 60), style)
        .draw(display)
        .unwrap();
    Text::new("OVER", Point::new(BOARD_X + 20, BOARD_Y + 75), style)
        .draw(display)
        .unwrap();

    let mut buf = [0u8; 16];
    let s = format_u32(score, &mut buf);
    Text::new(s, Point::new(BOARD_X + 10, BOARD_Y + 100), white)
        .draw(display)
        .unwrap();

    let mut buf2 = [0u8; 16];
    let l = format_u32(level as u32, &mut buf2);
    Text::new("Lv", Point::new(BOARD_X + 10, BOARD_Y + 115), white)
        .draw(display)
        .unwrap();
    Text::new(l, Point::new(BOARD_X + 26, BOARD_Y + 115), white)
        .draw(display)
        .unwrap();

    Text::new("START", Point::new(BOARD_X + 10, BOARD_Y + 135), white)
        .draw(display)
        .unwrap();
}

fn draw_pause(display: &mut Display) {
    Rectangle::new(
        Point::new(BOARD_X + 10, BOARD_Y + 65),
        Size::new(60, 20),
    )
    .into_styled(PrimitiveStyle::with_fill(BLACK))
    .draw(display)
    .unwrap();
    let style = MonoTextStyle::new(&FONT_6X10, Rgb565::YELLOW);
    Text::new("PAUSED", Point::new(BOARD_X + 14, BOARD_Y + 78), style)
        .draw(display)
        .unwrap();
}

// ── Tasks ───────────────────────────────────────────────────────────────────

#[embassy_executor::task]
async fn input_task(buttons: &'static mut Buttons) {
    info!("Tetris input task started");
    let mut prev_up = false;
    let mut prev_a = false;
    let mut prev_b = false;
    let mut prev_select = false;
    let mut prev_start = false;

    loop {
        let up = buttons.up.is_low();
        let a = buttons.a.is_low();
        let b = buttons.b.is_low();
        let select = buttons.select.is_high(); // select is pull-down, active high
        let start = buttons.start.is_low();

        INPUT_LEFT.store(buttons.left.is_low(), Ordering::Relaxed);
        INPUT_RIGHT.store(buttons.right.is_low(), Ordering::Relaxed);
        INPUT_DOWN.store(buttons.down.is_low(), Ordering::Relaxed);
        INPUT_UP.store(up, Ordering::Relaxed);
        INPUT_A.store(a, Ordering::Relaxed);
        INPUT_B.store(b, Ordering::Relaxed);
        INPUT_SELECT.store(select, Ordering::Relaxed);
        INPUT_START.store(start, Ordering::Relaxed);

        // Edge detection (rising edge = just pressed)
        if up && !prev_up {
            EDGE_UP.store(1, Ordering::Relaxed);
        }
        if a && !prev_a {
            EDGE_A.store(1, Ordering::Relaxed);
        }
        if b && !prev_b {
            EDGE_B.store(1, Ordering::Relaxed);
        }
        if select && !prev_select {
            EDGE_SELECT.store(1, Ordering::Relaxed);
        }
        if start && !prev_start {
            EDGE_START.store(1, Ordering::Relaxed);
        }

        prev_up = up;
        prev_a = a;
        prev_b = b;
        prev_select = select;
        prev_start = start;

        Timer::after(Duration::from_millis(8)).await;
    }
}

#[embassy_executor::task]
async fn led_task(leds: &'static mut Leds<'static>) {
    info!("Tetris LED task started");
    loop {
        let event = LED_CHANNEL.receive().await;
        match event {
            LedEvent::LineClear(lines) => {
                let color = match lines {
                    4 => Srgb::new(40u8, 40, 40), // Tetris = bright white
                    3 => Srgb::new(0, 30, 30),
                    2 => Srgb::new(0, 20, 0),
                    _ => Srgb::new(0, 0, 15),
                };
                for i in (0..=5).rev() {
                    let b = i as u8;
                    leds.fill(Srgb::new(
                        (color.red as u16 * b as u16 / 5) as u8,
                        (color.green as u16 * b as u16 / 5) as u8,
                        (color.blue as u16 * b as u16 / 5) as u8,
                    ));
                    leds.update().await;
                    Timer::after(Duration::from_millis(30)).await;
                }
            }
            LedEvent::TSpin => {
                for _ in 0..3 {
                    leds.fill(Srgb::new(30, 0, 30));
                    leds.update().await;
                    Timer::after(Duration::from_millis(80)).await;
                    leds.clear();
                    leds.update().await;
                    Timer::after(Duration::from_millis(80)).await;
                }
            }
            LedEvent::GameOver => {
                for _ in 0..4 {
                    leds.fill(Srgb::new(20, 0, 0));
                    leds.update().await;
                    Timer::after(Duration::from_millis(250)).await;
                    leds.clear();
                    leds.update().await;
                    Timer::after(Duration::from_millis(250)).await;
                }
            }
            LedEvent::LevelUp => {
                for i in 0..BAR_COUNT {
                    let mut bar = [Srgb::new(0u8, 0, 0); BAR_COUNT];
                    bar[i] = Srgb::new(0, 30, 10);
                    leds.set_both_bars(&bar);
                    leds.update().await;
                    Timer::after(Duration::from_millis(40)).await;
                }
                leds.clear();
                leds.update().await;
            }
        }
    }
}

#[embassy_executor::task]
async fn vibra_task(vibra: &'static mut Vibration) {
    info!("Tetris vibration task started");
    loop {
        let event = VIBRA_CHANNEL.receive().await;
        match event {
            VibraEvent::Drop => {
                vibra.pulse(Duration::from_millis(20)).await;
            }
            VibraEvent::LineClear => {
                vibra.pulse(Duration::from_millis(40)).await;
            }
            VibraEvent::Tetris => {
                vibra.pulse(Duration::from_millis(60)).await;
                Timer::after(Duration::from_millis(40)).await;
                vibra.pulse(Duration::from_millis(60)).await;
            }
        }
    }
}

#[embassy_executor::task]
async fn game_task(
    display: &'static mut Display<'static>,
    backlight: &'static mut Backlight,
) {
    backlight.on();
    info!("Tetris game started");

    loop {
        // Title screen
        draw_title(display);
        loop {
            if EDGE_START.swap(0, Ordering::Relaxed) > 0 {
                break;
            }
            Timer::after(Duration::from_millis(50)).await;
        }

        // Init game
        let mut game = Game::new();

        // Clear screen and draw static elements
        Rectangle::new(Point::zero(), Size::new(SCREEN_W as u32, SCREEN_H as u32))
            .into_styled(PrimitiveStyle::with_fill(BLACK))
            .draw(display)
            .unwrap();
        draw_board_border(display);
        draw_full_board(display, &game);
        draw_hud(display, &game);

        let mut prev_piece = game.piece;
        let mut prev_ghost_y = ghost_y(&game.board, &game.piece);
        let mut prev_board = game.board;
        let mut prev_score = game.score;
        let mut prev_level = game.level;
        let mut prev_hold = game.hold;
        let mut prev_next = game.bag.peek();

        let tick = Duration::from_millis(TICK_MS);

        // Game loop
        loop {
            // Pause toggle
            if EDGE_START.swap(0, Ordering::Relaxed) > 0 {
                if game.paused {
                    game.paused = false;
                    draw_full_board(display, &game);
                } else {
                    game.paused = true;
                    draw_pause(display);
                }
                Timer::after(Duration::from_millis(200)).await;
                continue;
            }

            if game.paused {
                Timer::after(tick).await;
                continue;
            }

            game.tick();

            // Incremental render
            draw_frame(display, &game, &prev_piece, prev_ghost_y, &prev_board);

            // Update HUD only when changed
            let next = game.bag.peek();
            if game.score != prev_score
                || game.level != prev_level
                || game.hold != prev_hold
                || next as u8 != prev_next as u8
            {
                draw_hud(display, &game);
                prev_score = game.score;
                prev_level = game.level;
                prev_hold = game.hold;
                prev_next = next;
            }

            prev_piece = game.piece;
            prev_ghost_y = ghost_y(&game.board, &game.piece);
            prev_board = game.board;

            if game.game_over {
                Timer::after(Duration::from_millis(300)).await;
                draw_game_over(display, game.score, game.level);
                LED_CHANNEL.try_send(LedEvent::GameOver).ok();

                // Wait for restart
                loop {
                    if EDGE_START.swap(0, Ordering::Relaxed) > 0 {
                        break;
                    }
                    Timer::after(Duration::from_millis(50)).await;
                }
                break;
            }

            Timer::after(tick).await;
        }
    }
}

#[esp_rtos::main]
async fn main(spawner: Spawner) -> ! {
    let peripherals = disobey2026badge::init();
    let resources = split_resources!(peripherals);

    esp_alloc::heap_allocator!(size: 64 * 1024);

    let timg0 = TimerGroup::new(peripherals.TIMG0);
    esp_rtos::start(timg0.timer0);

    let display = mk_static!(Display<'static>, resources.display.into());
    let backlight = mk_static!(Backlight, resources.backlight.into());
    let leds = mk_static!(Leds<'static>, resources.leds.into());
    let buttons = mk_static!(Buttons, resources.buttons.into());
    let vibra = mk_static!(Vibration, resources.vibra.into());

    spawner.must_spawn(input_task(buttons));
    spawner.must_spawn(led_task(leds));
    spawner.must_spawn(vibra_task(vibra));
    spawner.must_spawn(game_task(display, backlight));

    loop {
        Timer::after(Duration::from_secs(600)).await;
    }
}
