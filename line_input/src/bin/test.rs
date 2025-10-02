use line_input::InputEditor;
use line_input::LineInput;
use line_input::LineInputOptions;

#[cfg(not(tarpaulin_include))]
fn main() {
    let mut line = String::new();
    let mut reader = InputEditor::new();
    let res = reader.read(
        &mut line,
        &LineInputOptions { prompt: Some(':'), ..Default::default() },
    );
    match res {
        Err(e) => eprintln!("{e}"),
        Ok(bytes_read) => {
            eprintln!("read {bytes_read} bytes\n{line}");
        }
    }
}
