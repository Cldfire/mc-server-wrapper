use std::{
    collections::{BTreeMap, VecDeque},
    fmt::Display,
};

use chrono::{Duration, Local, TimeZone, Utc};
use crossterm::event::{Event, KeyCode};
use tui::{
    backend::Backend,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Style},
    text::{Span, Spans},
    widgets::{Block, Borders, List, ListItem, Paragraph, Row, Table, Tabs},
    Frame,
};
use unicode_width::UnicodeWidthStr;

use crate::OnlinePlayerInfo;

/// Represents the current state of the terminal UI
#[derive(Debug)]
pub struct TuiState {
    pub tab_state: TabsState,
    pub logs_state: LogsState,
    pub players_state: PlayersState,
}

impl TuiState {
    pub fn new() -> Self {
        TuiState {
            // TODO: don't hardcode this
            tab_state: TabsState::new(vec!["Logs".into(), "Players".into()]),
            logs_state: LogsState {
                records: VecDeque::with_capacity(512),
                progress_bar: None,
                input_state: InputState { value: "".into() },
            },
            players_state: PlayersState,
        }
    }

    /// Draw the current state to the given frame
    pub fn draw<B: Backend>(
        &mut self,
        f: &mut Frame<B>,
        online_players: &BTreeMap<String, OnlinePlayerInfo>,
    ) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(2), Constraint::Min(0)].as_ref())
            .split(f.size());

        self.tab_state.draw(f, chunks[0]);
        // TODO: create tab structs that report what index they belong at so this
        // isn't hardcoded
        match self.tab_state.current_idx {
            0 => self.logs_state.draw(f, chunks[1]),
            1 => self.players_state.draw(f, chunks[1], online_players),
            _ => unreachable!(),
        }
    }

    /// Update the state based on the given input
    // TODO: make input dispatch more generic
    pub fn handle_input(&mut self, event: Event) {
        self.tab_state.handle_input(event);
        match self.tab_state.current_idx {
            0 => self.logs_state.handle_input(event),
            1 => self.players_state.handle_input(event),
            _ => unreachable!(),
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
    fn new(titles: Vec<String>) -> Self {
        Self {
            titles,
            current_idx: 0,
        }
    }

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

    /// Update the state based on the given input
    fn handle_input(&mut self, event: Event) {
        if let Event::Key(key_event) = event {
            if key_event.code == KeyCode::Tab {
                self.next();
            } else if key_event.code == KeyCode::BackTab {
                self.previous();
            }
        }
    }

    /// Change to the next tab
    fn next(&mut self) {
        self.current_idx = (self.current_idx + 1) % self.titles.len();
    }

    /// Change to the previous tab
    fn previous(&mut self) {
        if self.current_idx > 0 {
            self.current_idx -= 1;
        } else {
            self.current_idx = self.titles.len() - 1;
        }
    }

    /// Get the current tab index
    // TODO: this being public is a hack
    pub fn current_idx(&self) -> usize {
        self.current_idx
    }
}

/// A simple struct to represent the state of a progress bar
///
/// This is used to display world loading progress bars in the logs
#[derive(Debug, PartialEq)]
struct ProgressBarState {
    complete: u32,
    out_of: u32,
}

impl Display for ProgressBarState {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.write_str("[")?;

        let num_progress_chars = ((self.complete as f32 / self.out_of as f32) * 10.0) as i32;
        let num_empty_chars = 10 - num_progress_chars;

        for _ in 0..num_progress_chars {
            f.write_str("=")?;
        }

        for _ in 0..num_empty_chars {
            f.write_str(" ")?;
        }

        f.write_fmt(format_args!("] {}%", self.complete))
    }
}

#[derive(Debug)]
#[allow(clippy::type_complexity)]
pub struct LogsState {
    /// Stores the log messages to be displayed
    ///
    /// (original_message, (wrapped_message, wrapped_at_width))
    records: VecDeque<(String, Option<(Vec<ListItem<'static>>, u16)>)>,
    /// The current state of the active progress bar (if present)
    progress_bar: Option<ProgressBarState>,
    /// State for the input (child widget)
    // TODO: this being public is a hack
    pub input_state: InputState,
}

impl LogsState {
    /// Draw the current state in the given `area`
    fn draw<B: Backend>(&mut self, f: &mut Frame<B>, area: Rect) {
        let (input_area, logs_area) = {
            let mut chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Min(0), Constraint::Length(2)].as_ref())
                .split(area);
            let input_area = chunks.pop().unwrap();
            let logs_area = chunks.pop().unwrap();

            (input_area, logs_area)
        };

        let available_lines = if self.progress_bar.is_some() {
            // Account for space needed for progress bar
            logs_area.height as usize - 1
        } else {
            logs_area.height as usize
        };
        let logs_area_width = logs_area.width as usize;

        let bar_string = if let Some(bar) = &self.progress_bar {
            bar.to_string()
        } else {
            String::new()
        };

        let num_records = self.records.len();
        // Keep track of the number of lines after wrapping so we can skip lines as
        // needed below
        let mut wrapped_lines_len = 0;

        let mut items = Vec::with_capacity(logs_area.height as usize);
        items.extend(
            self.records
                .iter_mut()
                // Only wrap the records we could potentially be displaying
                .skip(num_records.saturating_sub(available_lines))
                .map(|r| {
                    // See if we can use a cached wrapped line
                    if let Some(wrapped) = &r.1 {
                        if wrapped.1 as usize == logs_area_width {
                            wrapped_lines_len += wrapped.0.len();
                            return wrapped.0.clone();
                        }
                    }

                    // If not, wrap the line and cache it
                    r.1 = Some((
                        textwrap::wrap(r.0.as_ref(), logs_area_width)
                            .into_iter()
                            .map(|s| s.to_string())
                            .map(Span::from)
                            .map(ListItem::new)
                            .collect::<Vec<ListItem>>(),
                        logs_area.width,
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

        f.render_widget(logs, logs_area);
        self.input_state.draw(f, input_area);
    }

    /// Update the state based on the given input
    fn handle_input(&mut self, event: Event) {
        self.input_state.handle_input(event);
    }

    /// Add a record to be displayed
    pub fn add_record(&mut self, record: String) {
        self.records.push_back((record, None));
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
pub struct PlayersState;

impl PlayersState {
    /// Draw the current state in the given `area`
    fn draw<B: Backend>(
        &self,
        f: &mut Frame<B>,
        area: Rect,
        online_players: &BTreeMap<String, OnlinePlayerInfo>,
    ) {
        let now_utc = Utc::now();

        // TODO: doing all this work every draw for every online player is gonna
        // be bad with high player counts
        let online_players = online_players
            .iter()
            .map(|(n, info)| {
                let local_login_time = Local.from_utc_datetime(&info.joined_at.naive_utc());

                let session_time = now_utc - info.joined_at;
                let session_time_string = make_session_time_string(session_time);

                [
                    n.to_string(),
                    local_login_time.format("%r").to_string(),
                    session_time_string,
                ]
            })
            .collect::<Vec<_>>();

        let online_players = Table::new(
            ["Name", "Login Time", "Session Length"].iter(),
            online_players.iter().map(|d| Row::Data(d.iter())),
        )
        .block(Block::default().borders(Borders::NONE))
        .widths(&[
            Constraint::Length(16),
            Constraint::Length(11),
            Constraint::Length(14),
        ])
        .column_spacing(3);

        f.render_widget(online_players, area);
    }

    /// Update the state based on the given input
    fn handle_input(&mut self, _event: Event) {}
}

fn make_session_time_string(session_duration: Duration) -> String {
    let (session_minutes, session_hours, session_days) = (
        (session_duration - Duration::hours(session_duration.num_hours())).num_minutes(),
        (session_duration - Duration::days(session_duration.num_days())).num_hours(),
        session_duration.num_days(),
    );

    if session_hours == 0 {
        format!("{}m", session_minutes)
    } else if session_days == 0 {
        format!("{}h {}m", session_hours, session_minutes)
    } else {
        format!("{}d {}h {}m", session_days, session_hours, session_minutes)
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

    /// Update the state based on the given input
    fn handle_input(&mut self, event: Event) {
        if let Event::Key(key_event) = event {
            match key_event.code {
                KeyCode::Char(c) => self.value.push(c),
                KeyCode::Backspace => {
                    self.value.pop();
                }
                _ => {}
            }
        }
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

#[cfg(test)]
mod test {
    mod progress_bar {
        use crate::ui::ProgressBarState;

        #[test]
        fn progress_zero() {
            assert_eq!(
                ProgressBarState {
                    complete: 0,
                    out_of: 100
                }
                .to_string(),
                "[          ] 0%"
            );
        }

        #[test]
        fn progress_forty() {
            assert_eq!(
                ProgressBarState {
                    complete: 40,
                    out_of: 100
                }
                .to_string(),
                "[====      ] 40%"
            );
        }

        #[test]
        fn progress_one_hundred() {
            assert_eq!(
                ProgressBarState {
                    complete: 100,
                    out_of: 100
                }
                .to_string(),
                "[==========] 100%"
            );
        }
    }

    mod session_time_string {
        use chrono::Duration;

        use crate::ui::make_session_time_string;

        #[test]
        fn minutes() {
            assert_eq!(make_session_time_string(Duration::minutes(23)), "23m");
        }

        #[test]
        fn hours() {
            assert_eq!(
                make_session_time_string(Duration::hours(2) + Duration::minutes(12)),
                "2h 12m"
            );
        }

        #[test]
        fn days() {
            assert_eq!(
                make_session_time_string(
                    Duration::days(1) + Duration::hours(2) + Duration::minutes(12)
                ),
                "1d 2h 12m"
            );
        }
    }
}
