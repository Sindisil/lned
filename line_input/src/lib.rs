use std::cmp::Ordering;
use std::io::{self, prelude::*, Stdout};
use std::time::Duration;

use crossterm::{
    cursor,
    event::{self, Event, KeyCode, KeyEvent},
    terminal::{self, ClearType},
    ExecutableCommand, QueueableCommand,
};

#[derive(Debug)]
pub struct LineInput {
    history: Vec<String>,
}

impl Default for LineInput {
    fn default() -> Self {
        Self::new()
    }
}

impl LineInput {
    #[must_use]
    pub fn new() -> LineInput {
        LineInput {
            history: Vec::new(),
        }
    }

    pub fn read_line(prompt: &str) -> io::Result<String> {
        terminal::enable_raw_mode()?;

        let mut before_gap = String::new();
        let mut after_gap = String::new();
        let mut gap_start: usize = 0;

        let mut stdout = io::stdout();
        print!("{prompt}");
        let (begin_cur_col, _begin_cur_row) = cursor::position()?;

        loop {
            match event::read()? {
                Event::Resize(x, y) => {
                    let (_orig_sz, _new_sz) = flush_resize_events((x, y));
                    // todo: if new_sz col < orig_sz col, redisplay input text
                    //       first print before_gap, then capture cursor position
                    //       then print after_gap, then set cursor_position back to
                    //       saved value
                }
                Event::Key(KeyEvent {
                    code,
                    modifiers: _,
                    kind: _,
                    state: _,
                }) => match code {
                    KeyCode::Enter => {
                        println!();
                        stdout.execute(cursor::MoveToNextLine(1))?;
                        break;
                    }
                    KeyCode::Esc => {
                        before_gap.clear();
                        after_gap.clear();
                        stdout.execute(cursor::MoveToNextLine(1))?;
                        break;
                    }
                    KeyCode::Char(c) => {
                        move_gap(&mut before_gap, &mut after_gap, gap_start);
                        before_gap.push(c);
                        gap_start += 1;
                        stdout
                            .queue(cursor::SavePosition)?
                            .queue(cursor::Hide)?
                            .queue(terminal::Clear(ClearType::FromCursorDown))?;
                        print!("{c}{after_gap}");
                        stdout
                            .queue(cursor::RestorePosition)?
                            .queue(cursor::MoveRight(1))?
                            .queue(cursor::Show)?
                            .flush()?;
                    }
                    KeyCode::Backspace => {
                        if gap_start > 0 {
                            move_gap(&mut before_gap, &mut after_gap, gap_start);
                            gap_start -= 1;
                            before_gap.pop();
                            stdout
                                .queue(cursor::MoveLeft(1))?
                                .queue(cursor::SavePosition)?
                                .queue(terminal::Clear(ClearType::FromCursorDown))?;
                            print!("{after_gap}");
                            stdout.queue(cursor::RestorePosition)?.flush()?;
                        }
                    }
                    KeyCode::Left => {
                        if gap_start > 0 {
                            gap_start -= 1;
                            stdout.execute(cursor::MoveLeft(1))?;
                        }
                    }
                    KeyCode::Right => {
                        gap_start += 1;
                        stdout.execute(cursor::MoveRight(1))?;
                    }
                    KeyCode::Home => {
                        gap_start = 0;
                        stdout.execute(cursor::MoveToColumn(begin_cur_col))?;
                    }
                    _ => (),
                },
                event => println!("Event: {event:?}"),
            }
        }

        terminal::disable_raw_mode()?;

        Ok(format!("{before_gap}{after_gap}"))
    }
}

fn clear_input(
    before_gap: &mut String,
    after_gap: &mut String,
    stdout: &mut Stdout,
) -> io::Result<()> {
    before_gap.clear();
    after_gap.clear();
    todo!("move to beginning of input and clear it all")
}

fn move_gap(before_gap: &mut String, after_gap: &mut String, gap_start: usize) {
    match gap_start.cmp(&before_gap.len()) {
        Ordering::Less => {
            after_gap.insert_str(0, &before_gap[gap_start..]);
            before_gap.drain(gap_start..);
        }
        Ordering::Greater => {
            let to_move = gap_start - before_gap.len();
            before_gap.push_str(&after_gap[..to_move]);
            after_gap.drain(..to_move);
        }
        Ordering::Equal => (),
    }
}

fn flush_resize_events(first_resize: (u16, u16)) -> ((u16, u16), (u16, u16)) {
    let mut last_resize = first_resize;
    while let Ok(true) = event::poll(Duration::from_millis(50)) {
        if let Ok(Event::Resize(x, y)) = event::read() {
            last_resize = (x, y);
        }
    }

    (first_resize, last_resize)
}
