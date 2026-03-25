//! Teaser animation command - `co teaser`
//!
//! Shows a loading animation followed by title slides:
//! "Co" → "Cowork" → "Collaborate"

use std::f64::consts::PI;
use std::io::{self, Write};
use std::thread;
use std::time::{Duration, Instant};

// ANSI escape codes
const HIDE_CURSOR: &str = "\x1b[?25l";
const SHOW_CURSOR: &str = "\x1b[?25h";
const CLEAR_SCREEN: &str = "\x1b[2J";
const HOME: &str = "\x1b[H";
const DIM: &str = "\x1b[2m";
const BOLD: &str = "\x1b[1m";
const RESET: &str = "\x1b[0m";
const CYAN: &str = "\x1b[36m";

// Canvas dimensions (in braille dots: 2 dots wide x 4 dots tall per char)
const CHAR_WIDTH: usize = 32;
const CHAR_HEIGHT: usize = 16;
const DOT_WIDTH: usize = CHAR_WIDTH * 2;
const DOT_HEIGHT: usize = CHAR_HEIGHT * 4;

/// Cubic ease-out: fast start, slow finish
fn ease_out_cubic(t: f64) -> f64 {
    1.0 - (1.0 - t).powi(3)
}

/// Convert a 2x4 dot pattern to a braille character
fn dots_to_braille(dots: [[bool; 2]; 4]) -> char {
    // Braille dot positions:
    // 0 3
    // 1 4
    // 2 5
    // 6 7
    let mut code: u32 = 0x2800; // Braille base
    if dots[0][0] {
        code += 0x01;
    } // dot 1
    if dots[1][0] {
        code += 0x02;
    } // dot 2
    if dots[2][0] {
        code += 0x04;
    } // dot 3
    if dots[0][1] {
        code += 0x08;
    } // dot 4
    if dots[1][1] {
        code += 0x10;
    } // dot 5
    if dots[2][1] {
        code += 0x20;
    } // dot 6
    if dots[3][0] {
        code += 0x40;
    } // dot 7
    if dots[3][1] {
        code += 0x80;
    } // dot 8
    char::from_u32(code).unwrap_or(' ')
}

/// Check if angle is within the filled portion (pie-fill from 0°/right, counter-clockwise)
fn is_filled(angle: f64, progress: f64) -> bool {
    // Fill counter-clockwise: 0° (right) → 90° (up) → 180° (left) → 270° (down)
    let normalized = angle.rem_euclid(2.0 * PI);
    normalized <= progress * 2.0 * PI
}

/// Draw a thin circle outline with progress arc
fn draw_circle(progress: f64) -> Vec<String> {
    let mut canvas = vec![vec![false; DOT_WIDTH]; DOT_HEIGHT];

    // Circle parameters (accounting for terminal ~2:1 aspect ratio)
    let cx = DOT_WIDTH as f64 / 2.0;
    let cy = DOT_HEIGHT as f64 / 2.0;
    let rx = (DOT_WIDTH as f64 / 2.0) - 2.0; // horizontal radius
    let ry = (DOT_HEIGHT as f64 / 2.0) - 1.0; // vertical radius (smaller due to aspect)

    // Draw thin circle outline - only within progress arc
    let steps = 720;
    for i in 0..steps {
        let angle = (i as f64) * 2.0 * PI / (steps as f64);

        // Only draw if within progress
        if is_filled(angle, progress) {
            let x = cx + rx * angle.cos();
            let y = cy - ry * angle.sin();

            let px = x.round() as usize;
            let py = y.round() as usize;

            if px < DOT_WIDTH && py < DOT_HEIGHT {
                canvas[py][px] = true;
            }
        }
    }

    // Draw larger center dot (small filled circle)
    let dot_rx = 3.0; // radius in dots
    let dot_ry = 3.0;
    for i in 0..360 {
        let angle = (i as f64) * 2.0 * PI / 360.0;
        for r in 0..=3 {
            let ratio = r as f64 / 3.0;
            let fx = cx + (dot_rx * ratio) * angle.cos();
            let fy = cy - (dot_ry * ratio) * angle.sin();
            let fpx = fx.round() as usize;
            let fpy = fy.round() as usize;
            if fpx < DOT_WIDTH && fpy < DOT_HEIGHT {
                canvas[fpy][fpx] = true;
            }
        }
    }

    // Convert canvas to braille characters
    let mut lines = Vec::new();
    for row in (0..DOT_HEIGHT).step_by(4) {
        let mut line = String::new();
        for col in (0..DOT_WIDTH).step_by(2) {
            let mut dots = [[false; 2]; 4];
            for dy in 0..4 {
                for dx in 0..2 {
                    if row + dy < DOT_HEIGHT && col + dx < DOT_WIDTH {
                        dots[dy][dx] = canvas[row + dy][col + dx];
                    }
                }
            }
            line.push(dots_to_braille(dots));
        }
        lines.push(line);
    }

    lines
}

/// Draw the loading circle with progress
fn draw_loading(progress: f64) {
    let percent = (progress * 100.0) as u8;

    // Get the circle as braille art
    let circle_lines = draw_circle(progress);

    // Center content
    let center_text = if percent >= 100 {
        format!("{BOLD}{CYAN}⊙{RESET}") // Circled dot (monad symbol)
    } else {
        format!("{percent:>2}%")
    };

    print!("{CLEAR_SCREEN}{HOME}");
    println!();

    // Draw circle with percentage in center
    let mid_row = circle_lines.len() / 2;
    for (i, line) in circle_lines.iter().enumerate() {
        if i == mid_row {
            // Insert percentage in the middle
            let half = line.chars().count() / 2;
            let left: String = line.chars().take(half - 2).collect();
            let right: String = line.chars().skip(half + 2).collect();
            println!("    {DIM}{left}{RESET} {center_text} {DIM}{right}{RESET}");
        } else {
            println!("    {DIM}{line}{RESET}");
        }
    }

    io::stdout().flush().unwrap();
}

/// Display centered title - "Co" prefix is bold, rest is regular
fn draw_title(title: &str, highlight_co: bool) {
    print!("{CLEAR_SCREEN}{HOME}");
    println!();
    println!();
    println!();
    println!();
    println!();
    if highlight_co && title.starts_with("Co") {
        let rest = &title[2..];
        println!("              {BOLD}{CYAN}Co{RESET}{CYAN}{rest}{RESET}");
    } else {
        println!("              {BOLD}{CYAN}{title}{RESET}");
    }
    println!();
    println!();
    println!();
    io::stdout().flush().unwrap();
}

/// Draw the final monad symbol - thin circle outline + center dot (white)
fn draw_monad() {
    let mut canvas = vec![vec![false; DOT_WIDTH]; DOT_HEIGHT];

    let cx = DOT_WIDTH as f64 / 2.0;
    let cy = DOT_HEIGHT as f64 / 2.0;
    let rx = (DOT_WIDTH as f64 / 2.0) - 2.0;
    let ry = (DOT_HEIGHT as f64 / 2.0) - 1.0;

    // Draw thin circle outline only
    let steps = 720;
    for i in 0..steps {
        let angle = (i as f64) * 2.0 * PI / (steps as f64);
        let x = cx + rx * angle.cos();
        let y = cy - ry * angle.sin();
        let px = x.round() as usize;
        let py = y.round() as usize;
        if px < DOT_WIDTH && py < DOT_HEIGHT {
            canvas[py][px] = true;
        }
    }

    // Draw larger center dot (small filled circle)
    let dot_rx = 3.0;
    let dot_ry = 3.0;
    for i in 0..360 {
        let angle = (i as f64) * 2.0 * PI / 360.0;
        for r in 0..=3 {
            let ratio = r as f64 / 3.0;
            let fx = cx + (dot_rx * ratio) * angle.cos();
            let fy = cy - (dot_ry * ratio) * angle.sin();
            let fpx = fx.round() as usize;
            let fpy = fy.round() as usize;
            if fpx < DOT_WIDTH && fpy < DOT_HEIGHT {
                canvas[fpy][fpx] = true;
            }
        }
    }

    // Convert canvas to braille characters
    let mut lines = Vec::new();
    for row in (0..DOT_HEIGHT).step_by(4) {
        let mut line = String::new();
        for col in (0..DOT_WIDTH).step_by(2) {
            let mut dots = [[false; 2]; 4];
            for dy in 0..4 {
                for dx in 0..2 {
                    if row + dy < DOT_HEIGHT && col + dx < DOT_WIDTH {
                        dots[dy][dx] = canvas[row + dy][col + dx];
                    }
                }
            }
            line.push(dots_to_braille(dots));
        }
        lines.push(line);
    }

    print!("{CLEAR_SCREEN}{HOME}");
    println!();

    for line in &lines {
        println!("    {BOLD}{line}{RESET}");
    }

    io::stdout().flush().unwrap();
}

/// Run the teaser animation
pub fn run() {
    print!("{HIDE_CURSOR}");
    io::stdout().flush().unwrap();

    // Loading phase (~2 seconds with ease-out)
    let duration = Duration::from_millis(2000);
    let start = Instant::now();

    loop {
        let elapsed = start.elapsed();
        let t = (elapsed.as_secs_f64() / duration.as_secs_f64()).min(1.0);
        let progress = ease_out_cubic(t);

        draw_loading(progress);

        if t >= 1.0 {
            break;
        }
        thread::sleep(Duration::from_millis(33));
    }

    // Show completed monad (white)
    draw_monad();
    thread::sleep(Duration::from_millis(1000));

    // Title slides (1.5 seconds each, Co prefix bold)
    for title in ["Cowork", "Cocreate", "Collaborate", "Coordinate", "Conduct"] {
        draw_title(title, true);
        thread::sleep(Duration::from_millis(1500));
    }

    // Final frame: Q1 2026 (10 seconds)
    draw_title("Q1 2026", false);
    thread::sleep(Duration::from_millis(10000));

    print!("{CLEAR_SCREEN}{HOME}{SHOW_CURSOR}");
    io::stdout().flush().unwrap();
}
