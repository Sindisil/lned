use line_reader::LineRead;
use line_reader::LineReader;

fn main() {
    let mut line = String::new();
    let mut reader = LineReader::new();
    let res = reader.read_line(&mut line, ":");
    match res {
        Err(e) => eprintln!("{e}"),
        Ok(bytes_read) => {
            eprintln!("read {bytes_read} bytes\n{line}");
        }
    }
}
