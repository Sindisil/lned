use line_reader::LineRead;
use line_reader::LineReader;
use line_reader::LineReaderOptions;

#[cfg(not(tarpaulin_include))]
fn main() {
    let mut line = String::new();
    let mut reader = LineReader::new();
    let res = reader.read(
        &mut line,
        &LineReaderOptions { prompt: Some(':'), ..Default::default() },
    );
    match res {
        Err(e) => eprintln!("{e}"),
        Ok(bytes_read) => {
            eprintln!("read {bytes_read} bytes\n{line}");
        }
    }
}
