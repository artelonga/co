use clap::{Arg, ArgMatches, Command};
use obfstr::obfstr;

/// Commands parsed from CLI
pub enum Commands {
    Start,
    Vim,
    Update,
    Poker {
        room: Option<String>,
        create_room: bool,
        name: String,
        redis: Option<String>,
        chips: u32,
        small_blind: u32,
        big_blind: u32,
    },
    Profile,
    Balance,
    History {
        limit: usize,
    },
    Game,
}

pub fn build_cli() -> Command {
    Command::new(obfstr!("game").to_string())
        .about(obfstr!("A recursive CLI game engine").to_string())
        .subcommand_required(true)
        .subcommand(
            Command::new(obfstr!("start").to_string())
                .about(obfstr!("Start the game shell").to_string()),
        )
        .subcommand(
            Command::new(obfstr!("vim").to_string())
                .about(obfstr!("Start game in Vim mode").to_string()),
        )
        .subcommand(
            Command::new(obfstr!("update").to_string())
                .about(obfstr!("Update the game binary").to_string()),
        )
        .subcommand(
            Command::new(obfstr!("poker").to_string())
                .about(obfstr!("Play Texas Hold'em Poker").to_string())
                .arg(
                    Arg::new("room")
                        .short('r')
                        .long(obfstr!("room").to_string())
                        .help(obfstr!("Room ID to join").to_string()),
                )
                .arg(
                    Arg::new("create-room")
                        .short('c')
                        .long(obfstr!("create-room").to_string())
                        .action(clap::ArgAction::SetTrue)
                        .help(obfstr!("Create a new room").to_string()),
                )
                .arg(
                    Arg::new("name")
                        .short('n')
                        .long(obfstr!("name").to_string())
                        .default_value("Player")
                        .help(obfstr!("Player name").to_string()),
                )
                .arg(
                    Arg::new("redis")
                        .long(obfstr!("redis").to_string())
                        .env("REDIS_URL")
                        .help(obfstr!("Redis URL for multiplayer").to_string()),
                )
                .arg(
                    Arg::new("chips")
                        .long(obfstr!("chips").to_string())
                        .default_value("1000")
                        .value_parser(clap::value_parser!(u32))
                        .help(obfstr!("Starting chips").to_string()),
                )
                .arg(
                    Arg::new("small-blind")
                        .long(obfstr!("small-blind").to_string())
                        .default_value("10")
                        .value_parser(clap::value_parser!(u32))
                        .help(obfstr!("Small blind amount").to_string()),
                )
                .arg(
                    Arg::new("big-blind")
                        .long(obfstr!("big-blind").to_string())
                        .default_value("20")
                        .value_parser(clap::value_parser!(u32))
                        .help(obfstr!("Big blind amount").to_string()),
                ),
        )
        .subcommand(
            Command::new(obfstr!("profile").to_string())
                .about(obfstr!("Show player profile and stats").to_string()),
        )
        .subcommand(
            Command::new(obfstr!("balance").to_string())
                .about(obfstr!("Show wallet balance").to_string()),
        )
        .subcommand(
            Command::new(obfstr!("history").to_string())
                .about(obfstr!("Show hand history").to_string())
                .arg(
                    Arg::new("limit")
                        .short('n')
                        .long(obfstr!("limit").to_string())
                        .default_value("20")
                        .value_parser(clap::value_parser!(usize))
                        .help(obfstr!("Number of hands to show").to_string()),
                ),
        )
        .subcommand(
            Command::new(obfstr!("game").to_string()).about(obfstr!("Play the game").to_string()),
        )
}

pub fn parse_commands(matches: ArgMatches) -> Commands {
    match matches.subcommand() {
        Some(("start", _)) => Commands::Start,
        Some(("vim", _)) => Commands::Vim,
        Some(("update", _)) => Commands::Update,
        Some(("poker", sub)) => Commands::Poker {
            room: sub.get_one::<String>("room").cloned(),
            create_room: sub.get_flag("create-room"),
            name: sub
                .get_one::<String>("name")
                .cloned()
                .unwrap_or_else(|| "Player".into()),
            redis: sub.get_one::<String>("redis").cloned(),
            chips: sub.get_one::<u32>("chips").copied().unwrap_or(1000),
            small_blind: sub.get_one::<u32>("small-blind").copied().unwrap_or(10),
            big_blind: sub.get_one::<u32>("big-blind").copied().unwrap_or(20),
        },
        Some(("profile", _)) => Commands::Profile,
        Some(("balance", _)) => Commands::Balance,
        Some(("history", sub)) => Commands::History {
            limit: sub.get_one::<usize>("limit").copied().unwrap_or(20),
        },
        Some(("game", _)) => Commands::Game,
        _ => std::process::exit(1),
    }
}
