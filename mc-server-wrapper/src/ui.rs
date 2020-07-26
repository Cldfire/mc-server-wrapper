use crossterm::event::{Event, KeyCode};
use ringbuffer::RingBuffer;
use textwrap::Wrapper;
use tui::{
    backend::Backend,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Style},
    text::{Span, Spans},
    widgets::{Block, Borders, List, ListItem, Paragraph, Tabs},
    Frame,
};
use unicode_width::UnicodeWidthStr;

/// Represents the current state of the terminal UI
#[derive(Debug)]
pub struct TuiState {
    pub tab_state: TabsState,
    pub logs_state: LogsState,
    pub input_state: InputState,
}

impl TuiState {
    pub fn new() -> Self {
        TuiState {
            tab_state: TabsState::new(vec!["Logs".into()]),
            logs_state: LogsState {
                records: RingBuffer::with_capacity(512),
                progress_bar: None,
            },
            input_state: InputState { value: "".into() },
        }
    }

    /// Draw the current state to the given frame
    pub fn draw<B: Backend>(&mut self, f: &mut Frame<B>) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints(
                [
                    Constraint::Length(2),
                    Constraint::Min(0),
                    Constraint::Length(2),
                ]
                .as_ref(),
            )
            .split(f.size());

        self.tab_state.draw(f, chunks[0]);
        self.logs_state.draw(f, chunks[1]);
        self.input_state.draw(f, chunks[2]);
    }

    /// Update the state based on the given input
    // TODO: have handle_input for each state struct?
    pub fn handle_input(&mut self, event: Event) {
        if let Event::Key(key_event) = event {
            match key_event.code {
                KeyCode::Char(c) => self.input_state.value.push(c),
                KeyCode::Backspace => {
                    self.input_state.value.pop();
                }
                _ => {}
            }
        }
    }
}

#[derive(Debug)]
pub struct TabsState {
    /// List of tab titles
    titles: Vec<String>,
    /// The currently active tab
    current_idx: usize,
}

impl TabsState {
    /// Draw the current state in the given `area`
    fn draw<B: Backend>(&self, f: &mut Frame<B>, area: Rect) {
        let tabs = Tabs::new(
            self.titles
                .iter()
                .map(|s| s.as_ref())
                .map(Spans::from)
                .collect(),
        )
        .block(Block::default().borders(Borders::BOTTOM))
        .highlight_style(Style::default().fg(Color::Yellow))
        .select(self.current_idx);

        f.render_widget(tabs, area);
    }

    fn new(titles: Vec<String>) -> Self {
        Self {
            titles,
            current_idx: 0,
        }
    }

    // /// Change to the next tab
    // pub fn next(&mut self) {
    //     self.current_idx = (self.current_idx + 1) % self.titles.len();
    // }

    // /// Change to the previous tab
    // pub fn previous(&mut self) {
    //     if self.current_idx > 0 {
    //         self.current_idx -= 1;
    //     } else {
    //         self.current_idx = self.titles.len() - 1;
    //     }
    // }
}

/// A simple struct to represent the state of a progress bar
///
/// This is used to display world loading progress bars in the logs
#[derive(Debug, PartialEq)]
struct ProgressBarState {
    complete: u32,
    out_of: u32,
}

impl ProgressBarState {
    /// Create a string for displaying the progress bar
    fn to_string(&self) -> String {
        let mut output = String::with_capacity(20);
        output.push('[');

        let num_progress_chars = ((self.complete as f32 / self.out_of as f32) * 10.0) as i32;
        let num_empty_chars = 10 - num_progress_chars;

        for _ in 0..num_progress_chars {
            output.push('=');
        }

        for _ in 0..num_empty_chars {
            output.push(' ');
        }

        output.push_str(&format!("] {}%", self.complete));

        output
    }
}

#[derive(Debug)]
pub struct LogsState {
    /// Stores the log messages to be displayed
    ///
    /// (original_message, (wrapped_message, wrapped_at_width))
    records: RingBuffer<(String, Option<(Vec<ListItem<'static>>, u16)>)>,
    /// The current state of the active progress bar (if present)
    progress_bar: Option<ProgressBarState>,
}

impl LogsState {
    /// Draw the current state in the given `area`
    fn draw<B: Backend>(&mut self, f: &mut Frame<B>, area: Rect) {
        let available_lines = if self.progress_bar.is_some() {
            // Account for space needed for progress bar
            area.height as usize - 1
        } else {
            area.height as usize
        };
        let area_width = area.width as usize;

        let bar_string = if let Some(bar) = &self.progress_bar {
            bar.to_string()
        } else {
            String::new()
        };

        let wrapper = Wrapper::new(area_width);
        let num_records = self.records.len();
        // Keep track of the number of lines after wrapping so we can skip lines as
        // needed below
        let mut wrapped_lines_len = 0;

        let mut items = Vec::with_capacity(area.height as usize);
        items.extend(
            self.records
                .iter_mut()
                // Only wrap the records we could potentially be displaying
                .skip(num_records.saturating_sub(available_lines))
                .map(|r| {
                    // See if we can use a cached wrapped line
                    if let Some(wrapped) = &r.1 {
                        if wrapped.1 as usize == area_width {
                            wrapped_lines_len += wrapped.0.len();
                            return wrapped.0.clone();
                        }
                    }

                    // If not, wrap the line and cache it
                    *(&mut r.1) = Some((
                        wrapper
                            .wrap(r.0.as_ref())
                            .into_iter()
                            .map(|s| s.to_string())
                            .map(Span::from)
                            .map(ListItem::new)
                            .collect::<Vec<ListItem>>(),
                        area.width,
                    ));

                    wrapped_lines_len += r.1.as_ref().unwrap().0.len();
                    r.1.as_ref().unwrap().0.clone()
                })
                .flatten(),
        );

        if self.progress_bar.is_some() {
            items.push(ListItem::new(bar_string.as_str()));
        }

        // TODO: we should be wrapping text with paragraph, but it currently
        // doesn't support wrapping and staying scrolled to the bottom
        //
        // see https://github.com/fdehau/tui-rs/issues/89
        let logs = List::new(
            items
                .into_iter()
                // Wrapping could have created more lines than what we can display;
                // skip them
                .skip(wrapped_lines_len.saturating_sub(available_lines))
                .collect::<Vec<_>>(),
        )
        .block(Block::default().borders(Borders::NONE));
        f.render_widget(logs, area);
    }

    /// Add a record to be displayed
    pub fn add_record(&mut self, record: String) {
        self.records.push((record, None));
    }

    /// Set the progress bar to the given percentage of completion
    ///
    /// Setting to 100 clears the bar
    pub fn set_progress_percent(&mut self, percent: u32) {
        match self.progress_bar.as_mut() {
            Some(bar) => {
                if percent >= 100 {
                    self.progress_bar = None;
                } else {
                    bar.complete = percent;
                }
            }
            None => {
                self.progress_bar = Some(ProgressBarState {
                    complete: percent,
                    out_of: 100,
                })
            }
        }
    }
}

#[derive(Debug)]
pub struct InputState {
    /// The current value of the input
    value: String,
}

impl InputState {
    /// Draw the current state in the given `area`
    fn draw<B: Backend>(&self, f: &mut Frame<B>, area: Rect) {
        let text = Spans::from(vec![Span::raw("> "), Span::raw(&self.value)]);
        let value_width = self.value.width() as u16;

        let input = Paragraph::new(text)
            .style(Style::default().fg(Color::Yellow))
            .block(Block::default().borders(Borders::NONE));

        f.render_widget(input, area);
        f.set_cursor(value_width + 2, area.y);
    }

    /// Clear the input
    pub fn clear(&mut self) {
        self.value.clear();
    }

    /// Get the current value of the input
    pub fn value(&self) -> &str {
        &self.value
    }
}
