fn main() {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("update") => {
            let filter = args.get(2).map_or("all", String::as_str);
            if let Err(err) = clux::run_update(filter) {
                eprintln!("clux: {err}");
                std::process::exit(1);
            }
        }
        Some("list") => {
            if let Err(err) = clux::run_list() {
                eprintln!("clux: {err}");
                std::process::exit(1);
            }
        }
        Some("select") => {
            let filter = args.get(2).map_or("all", String::as_str);
            if let Err(err) = clux::run_select(filter) {
                eprintln!("clux: {err}");
                std::process::exit(1);
            }
        }
        Some("pick") => {
            if let Err(err) = clux::run_pick() {
                eprintln!("clux: {err}");
                std::process::exit(1);
            }
        }
        _ => {
            eprintln!("Usage: clux <update [filter]|list|select [filter]|pick>");
            std::process::exit(1);
        }
    }
}
