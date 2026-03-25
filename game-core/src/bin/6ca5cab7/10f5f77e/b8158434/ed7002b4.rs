//! Movie content — edit this file to change what plays.
//!
//! Each scene plays sequentially. Within a scene, acts play in order.
//!
//! Available acts:
//!   typew(row, col, text, ms_per_word, style) — typewriter at position
//!   center(row, text, ms_per_word, style)     — centered typewriter
//!   print_at(row, col, text, style)           — instant text
//!   wait(ms)                                  — pause
//!   fade(ms)                                  — fade out and clear
//!
//! Styles: bold(color), dim(color), italic(color), plain(color)
//! Colors: yellow(), cyan(), green(), red(), white(), grey(), magenta()

use obfstr::obfstr;

use super::types::*;

/// The teaser movie — cryptic game hints, animated.
pub fn teaser() -> Movie {
    Movie::from(vec![
        // ══════════════ TITLE ══════════════
        scene(vec![
            wait(600),
            center(10, obfstr!("G  A  M  E"), 350, bold(yellow())),
            wait(2500),
            fade(800),
        ]),
        // ══════════════ HINT 1 ══════════════
        scene(vec![
            wait(400),
            center(9, obfstr!("pst knock"), 250, bold(cyan())),
            wait(800),
            center(11, obfstr!("(knock)"), 200, dim(grey())),
            wait(1200),
            center(14, obfstr!("?"), 0, dim(white())),
            wait(2000),
            fade(600),
        ]),
        // ══════════════ HINT 2 ══════════════
        scene(vec![
            wait(400),
            center(8, obfstr!("pst bom-dia"), 250, bold(cyan())),
            wait(400),
            center(9, obfstr!("e"), 0, dim(grey())),
            wait(200),
            center(10, obfstr!("pst good-morning"), 250, bold(cyan())),
            wait(600),
            center(12, obfstr!("mean the same thing?"), 150, italic(white())),
            wait(800),
            center(14, obfstr!("(knock knock)"), 200, dim(grey())),
            wait(1200),
            center(16, obfstr!("?"), 0, dim(white())),
            wait(2000),
            fade(600),
        ]),
        // ══════════════ HINT 3 ══════════════
        scene(vec![
            wait(400),
            center(
                10,
                obfstr!("https://vim-adventures.com"),
                200,
                bold(green()),
            ),
            wait(1000),
            center(12, obfstr!("(knock knock knock)"), 180, dim(grey())),
            wait(1200),
            center(14, obfstr!("?"), 0, dim(white())),
            wait(2000),
            fade(600),
        ]),
        // ══════════════ DATE ══════════════
        scene(vec![
            wait(600),
            center(10, obfstr!("NOT January 17"), 300, bold(red())),
            wait(1200),
            center(12, obfstr!("(Slack)"), 250, dim(grey())),
            wait(2500),
            fade(800),
        ]),
        // ══════════════ END ══════════════
        scene(vec![
            wait(800),
            center(12, obfstr!("..."), 500, dim(white())),
            wait(3000),
            fade(600),
        ]),
    ])
}
