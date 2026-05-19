fn main() {
    let mut args = std::env::args().skip(1);

    match args.next().as_deref() {
        Some("filter") => {
            let input = args.collect::<Vec<_>>().join(" ");
            let result = mizan_rtk::passthrough_filter(input);
            println!("{}", result.body);
        }
        _ => {
            println!("mizan-cli");
            println!("usage: mizan-cli filter <text>");
        }
    }
}
