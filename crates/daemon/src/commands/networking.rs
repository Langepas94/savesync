use super::*;
use savesync_core::lan::LanAction;
pub(super) fn command(args: &[String]) -> i32 {
    let action = match args.first().map(String::as_str) {
        Some("status") if args.len() == 1 => LanAction::Status,
        Some("discover") if args.len() == 1 => LanAction::Discover,
        Some("invite") if args.len() == 1 => LanAction::Invite,
        Some("cancel") if args.len() == 1 => LanAction::ClosePairing,
        Some("pair") if args.len() == 3 => LanAction::Pair {
            address: args[1].clone(),
            pin: args[2].clone(),
        },
        Some("confirm") if args.len() == 2 => LanAction::Confirm {
            peer_id: args[1].clone(),
        },
        Some("forget") if args.len() == 2 => LanAction::Forget {
            peer_id: args[1].clone(),
        },
        _ => {
            eprintln!("remote lan status|discover|invite|cancel|pair <host:port> <PIN>|confirm <peer_id>|forget <peer_id>");
            return 2;
        }
    };
    cmd_via_daemon(Method::Lan { action })
}
