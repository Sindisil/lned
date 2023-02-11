mod cli;

fn main() {
    let args = match cli::parse_args() {
        Ok(args) => args,
        Err(err) => {
            eprintln!("Error: {err}");
            std::process::exit(1);
        }
    };
    println!("{args:#?}");
}
