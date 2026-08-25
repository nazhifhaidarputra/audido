use anyhow::Ok;
use audido_core::{
    dsp::eq::{Equalizer, FilterNode},
    modules::{self, core::CoreHandle},
};
use ratatui::{
    Frame,
    crossterm::event::KeyCode,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    symbols,
    text::{Line, Span},
    widgets::{
        Block, Borders, List, ListItem, Paragraph,
        canvas::{Canvas, Context as CanvasContext, Line as CanvasLine, Points},
    },
};
use strum::VariantArray;

use crate::{
    router::{InterceptKeyResult, RouteAction, RouteHandler, get_next_tab, route_for_name},
    state::AppState,
    states::{AudioState, EqMode, EqState},
    ui::{draw_generic_dialog, open_modal},
};

#[derive(PartialEq, Clone, Copy, Debug)]
pub enum EqFocus {
    /// Curve/Graph panel - up/down controls master gain
    CurvePanel,
    /// Band panel - up/down selects bands (Advanced mode only)
    BandPanel,
}

#[derive(Debug, Clone)]
pub struct BandFilterConfig {
    pub selected_band: usize,
    pub option: Vec<String>,
    pub selected_param: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, VariantArray)]
pub enum EqDialogOption {
    EditBand,
    ResetBand,
}

impl EqDialogOption {
    /// Cycle to the next option
    pub fn next(&self) -> Self {
        let index = self.index();
        let next_index = (index + 1) % Self::VARIANTS.len();
        Self::VARIANTS[next_index]
    }

    /// Cycle to the previous option
    pub fn prev(&self) -> Self {
        let index = self.index();
        let prev_index = (index + Self::VARIANTS.len() - 1) % Self::VARIANTS.len();
        Self::VARIANTS[prev_index]
    }

    pub fn index(self) -> usize {
        self as usize
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EqDialogState {
    None,
    EqBandSelect {
        selected_band: usize,
        selected_dialog_option: EqDialogOption,
    },
}

/// Equalizer route
#[derive(Debug, Clone)]
pub struct EqualizerRoute {
    // Local UI state
    eq_focus: EqFocus,
    eq_selected_band: usize,
    eq_dialog_state: EqDialogState,
    eq_filter_band_config_opened: Option<BandFilterConfig>,

    /// Tell whether it is in changing param value state
    locked_in: bool,
}

impl Default for EqualizerRoute {
    fn default() -> Self {
        Self {
            eq_focus: EqFocus::CurvePanel,
            eq_selected_band: 0,
            eq_dialog_state: EqDialogState::None,
            eq_filter_band_config_opened: None,
            locked_in: false,
        }
    }
}

impl EqualizerRoute {
    // Focus

    /// Toggle focus between CurvePanel and BandPanel
    fn toggle_focus(&mut self) {
        self.eq_focus = match self.eq_focus {
            EqFocus::CurvePanel => EqFocus::BandPanel,
            EqFocus::BandPanel => EqFocus::CurvePanel,
        };
    }

    // Band selection

    /// Select next band in the filter list
    fn next_band(&mut self, num_filters: usize) {
        if num_filters > 0 {
            self.eq_selected_band = (self.eq_selected_band + 1) % num_filters;
        }
    }

    /// Select previous band in the filter list
    fn prev_band(&mut self, num_filters: usize) {
        if num_filters > 0 {
            self.eq_selected_band = if self.eq_selected_band == 0 {
                num_filters - 1
            } else {
                self.eq_selected_band - 1
            };
        }
    }

    /// Navigate next: cycles dialog option when dialog is open, else selects next band
    fn next(&mut self, num_filters: usize) {
        match &mut self.eq_dialog_state {
            EqDialogState::None => {
                self.next_band(num_filters);
            }
            EqDialogState::EqBandSelect {
                selected_dialog_option,
                ..
            } => {
                *selected_dialog_option = selected_dialog_option.next();
            }
        }
    }

    /// Navigate prev: cycles dialog option when dialog is open, else selects prev band
    fn prev(&mut self, num_filters: usize) {
        match &mut self.eq_dialog_state {
            EqDialogState::None => {
                self.prev_band(num_filters);
            }
            EqDialogState::EqBandSelect {
                selected_dialog_option,
                ..
            } => {
                *selected_dialog_option = selected_dialog_option.prev();
            }
        }
    }

    // Dialog / modal

    fn open_filter_band_dialog(&mut self) {
        self.eq_dialog_state = EqDialogState::EqBandSelect {
            selected_band: self.eq_selected_band,
            selected_dialog_option: EqDialogOption::EditBand,
        };
    }

    fn close_filter_band_dialog(&mut self) {
        self.eq_dialog_state = EqDialogState::None;
    }

    fn open_filter_band_config(&mut self, config: BandFilterConfig) {
        self.eq_filter_band_config_opened = Some(config);
    }

    fn close_filter_band_config(&mut self) {
        self.eq_filter_band_config_opened = None;
    }

    /// Navigate to the next param in the config modal
    fn next_config_param(&mut self) {
        if let Some(config) = &mut self.eq_filter_band_config_opened
            && !config.option.is_empty()
        {
            config.selected_param = (config.selected_param + 1) % config.option.len();
        }
    }

    /// Navigate to the previous param in the config modal
    fn prev_config_param(&mut self) {
        if let Some(config) = &mut self.eq_filter_band_config_opened
            && !config.option.is_empty()
        {
            config.selected_param = if config.selected_param == 0 {
                config.option.len() - 1
            } else {
                config.selected_param - 1
            };
        }
    }

    // Input handlers

    /// Handle input when the filter band config modal is open
    fn handle_filter_band_config_input(
        &mut self,
        key: KeyCode,
        state: &mut AppState,
        handle: &CoreHandle,
    ) -> anyhow::Result<RouteAction> {
        match key {
            KeyCode::Up => {
                if !self.locked_in {
                    self.prev_config_param();
                }
            }
            KeyCode::Down => {
                if !self.locked_in {
                    self.next_config_param();
                }
            }
            KeyCode::Esc => {
                self.close_filter_band_config();
                self.locked_in = false;
            }
            KeyCode::Enter => {
                if self.eq_filter_band_config_opened.is_some() {
                    self.locked_in = !self.locked_in;
                }
            }
            KeyCode::Left => {
                if self.locked_in {
                    self.handle_set_parameter(state, handle, false)?;
                }
            }
            KeyCode::Right => {
                if self.locked_in {
                    self.handle_set_parameter(state, handle, true)?;
                }
            }
            _ => {}
        }
        Ok(RouteAction::None)
    }

    fn handle_set_parameter(
        &mut self,
        state: &mut AppState,
        handle: &CoreHandle,
        is_increment: bool,
    ) -> anyhow::Result<RouteAction> {
        if let Some(config) = &mut self.eq_filter_band_config_opened {
            // get this mutable ref of filter node
            let Some(filter_node) = state.eq.local_filters.get_mut(config.selected_band) else {
                return Ok(RouteAction::None);
            };

            match config.selected_param {
                0 => {
                    filter_node.set_filter_type(if is_increment {
                        filter_node.filter_type.next()
                    } else {
                        filter_node.filter_type.prev()
                    });
                }
                1 => {
                    let delta = if is_increment { 10.0 } else { -10.0 };
                    filter_node.set_freq(filter_node.freq + delta);
                }
                2 => {
                    let delta = if is_increment { 0.5 } else { -0.5 };
                    filter_node.set_gain(filter_node.gain + delta);
                }
                3 => {
                    let delta = if is_increment { 0.1 } else { -0.1 };
                    filter_node.set_q_factor(filter_node.q + delta);
                }
                4 => {
                    let new_order = if is_increment {
                        filter_node.order.saturating_add(1)
                    } else {
                        filter_node.order.saturating_sub(1).max(1)
                    };
                    filter_node.set_order(new_order);
                }
                _ => {}
            }

            // Send updated filters to audio core
            modules::dsp::set_filters(handle.ctx(), state.eq.local_filters.clone());
        }
        Ok(RouteAction::None)
    }
    /// Handle input when the band select dialog is open
    fn handle_band_select_dialog_input(
        &mut self,
        key: KeyCode,
        state: &mut AppState,
        handle: &CoreHandle,
    ) -> anyhow::Result<RouteAction> {
        let num_filters = state.eq.local_filters.len();
        match key {
            KeyCode::Up => {
                self.prev(num_filters);
            }
            KeyCode::Down => {
                self.next(num_filters);
            }
            KeyCode::Enter => {
                if let EqDialogState::EqBandSelect {
                    selected_band,
                    selected_dialog_option,
                } = self.eq_dialog_state
                {
                    self.close_filter_band_dialog();

                    match selected_dialog_option {
                        EqDialogOption::EditBand => {
                            self.open_filter_band_config(BandFilterConfig {
                                selected_band,
                                option: vec![
                                    "Type".to_string(),
                                    "Frequency".to_string(),
                                    "Gain".to_string(),
                                    "Q Factor".to_string(),
                                    "Order".to_string(),
                                ],
                                selected_param: 0,
                            });
                        }
                        EqDialogOption::ResetBand => {
                            // Reset the local filter to preset default
                            let preset_filters = state.eq.local_preset.set_filters();
                            if let Some(default_node) = preset_filters.get(selected_band).cloned() {
                                if let Some(filter) = state.eq.local_filters.get_mut(selected_band)
                                {
                                    *filter = default_node;
                                }
                            }
                            // Notify audio core
                            modules::dsp::reset_filter_node(handle.ctx(), selected_band);
                        }
                    }
                }
            }
            KeyCode::Esc => {
                self.close_filter_band_dialog();
            }
            _ => {}
        }
        Ok(RouteAction::None)
    }

    /// Handle input when no floating panel is open (default state)
    fn handle_default_input(
        &mut self,
        key: KeyCode,
        state: &mut AppState,
        handle: &CoreHandle,
    ) -> anyhow::Result<RouteAction> {
        let num_filters = state.eq.local_filters.len();
        match key {
            KeyCode::Left | KeyCode::Right => {
                self.toggle_focus();
            }
            KeyCode::Up => {
                match self.eq_focus {
                    EqFocus::CurvePanel => {
                        state.eq.adjust_master_gain(0.5);
                        modules::dsp::set_master_gain(handle.ctx(), state.eq.local_master_gain);
                    }
                    EqFocus::BandPanel => {
                        match state.eq.eq_mode {
                            EqMode::Casual => {
                                // TODO: implement toggle preset
                            }
                            EqMode::Advanced => {
                                self.prev(num_filters);
                            }
                        }
                    }
                }
            }
            KeyCode::Down => match self.eq_focus {
                EqFocus::CurvePanel => {
                    state.eq.adjust_master_gain(-0.5);
                    modules::dsp::set_master_gain(handle.ctx(), state.eq.local_master_gain);
                }
                EqFocus::BandPanel => {
                    self.next(num_filters);
                }
            },
            KeyCode::Char('t') => {
                state.eq.toggle_enabled();
                modules::dsp::set_enabled(handle.ctx(), state.eq.eq_enabled);
            }
            KeyCode::Char('m') => {
                state.eq.toggle_mode();
            }
            KeyCode::Char('a') => {
                if state.eq.local_filters.len() < 8 {
                    let new_id = state.eq.local_filters.len() as i16;
                    let new_filter = FilterNode::new(new_id, 1000.0);
                    state.eq.local_filters.push(new_filter);
                    modules::dsp::set_filters(handle.ctx(), state.eq.local_filters.clone());
                }
            }
            KeyCode::Enter => {
                if self.eq_focus == EqFocus::BandPanel && state.eq.eq_mode == EqMode::Advanced {
                    self.open_filter_band_dialog();
                }
            }
            _ => {}
        }
        Ok(RouteAction::None)
    }

    fn has_floating_panel(&self) -> bool {
        self.eq_filter_band_config_opened.is_some() || self.eq_dialog_state != EqDialogState::None
    }
}

impl RouteHandler for EqualizerRoute {
    fn render(&mut self, frame: &mut Frame, area: Rect, state: &AppState) {
        draw_eq_panel(
            frame,
            area,
            &state.eq,
            &state.audio,
            self.eq_focus,
            self.eq_selected_band,
        );

        if let EqDialogState::EqBandSelect {
            selected_band,
            selected_dialog_option,
        } = &self.eq_dialog_state
        {
            let title = format!(" Edit Band {} ", selected_band + 1);
            let props = crate::ui::DialogProperties {
                title: title.as_str(),
                options: vec!["Edit", "Reset Parameteres"],
                selected_index: selected_dialog_option.index(),
            };
            draw_generic_dialog(frame, area, props);
        }

        // draw filter band configuration modal
        if let Some(config) = &self.eq_filter_band_config_opened {
            let locked = self.locked_in;
            open_modal(frame, area, (&state.eq, config), |f, area, (eq, cfg)| {
                draw_filter_band_config_modal(f, area, eq, cfg, locked);
            });
        }
    }

    fn handle_input(
        &mut self,
        key: KeyCode,
        state: &mut AppState,
        handle: &CoreHandle,
    ) -> anyhow::Result<RouteAction> {
        // State-first dispatch: route to the handler for the active UI context
        if self.eq_filter_band_config_opened.is_some() {
            return self.handle_filter_band_config_input(key, state, handle);
        }

        if let EqDialogState::EqBandSelect { .. } = self.eq_dialog_state {
            return self.handle_band_select_dialog_input(key, state, handle);
        }

        self.handle_default_input(key, state, handle)
    }

    fn name(&self) -> &str {
        "Equalizer"
    }

    fn on_enter(&mut self, _state: &mut AppState, _handle: &CoreHandle) -> anyhow::Result<()> {
        Ok(())
    }

    fn on_exit(&mut self, _state: &mut AppState, _handle: &CoreHandle) -> anyhow::Result<()> {
        Ok(())
    }

    fn intercept_global_key(
        &mut self,
        key: KeyCode,
        state: &mut AppState,
        handle: &CoreHandle,
    ) -> InterceptKeyResult {
        // If any floating panel is open, intercept ALL keys and delegate to handle_input
        if self.has_floating_panel() {
            // Delegate to handle_input which already does state-first dispatch
            let _ = self.handle_input(key, state, handle);
            return InterceptKeyResult::Handled;
        }

        if key == KeyCode::Tab {
            let Some(next_tab_name) = get_next_tab("Settings") else {
                return InterceptKeyResult::Ignored;
            };

            // should clear route and then go to next from setting router
            return InterceptKeyResult::HandledAndNavigate(RouteAction::Reset(route_for_name(
                next_tab_name,
            )));
        }
        InterceptKeyResult::Ignored
    }
}

pub fn draw_eq_panel(
    f: &mut Frame,
    area: Rect,
    eq_state: &EqState,
    audio_state: &AudioState,
    eq_focus: EqFocus,
    eq_selected_band: usize,
) {
    let block = Block::default()
        .title(" Equalizer ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Magenta));

    let inner = block.inner(area);
    f.render_widget(block, area);

    // Split EQ panel: Mode Toggle, EQ Graph, and Controls
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // Mode toggle row
            Constraint::Min(8),    // EQ Graph
            Constraint::Length(6), // EQ Controls
        ])
        .split(inner);

    draw_eq_mode_toggle(f, chunks[0], eq_state);
    draw_eq_graph(
        f,
        chunks[1],
        eq_state,
        audio_state,
        eq_focus,
        eq_selected_band,
    );
    draw_eq_controls(f, chunks[2], eq_state, eq_focus, eq_selected_band);
}

fn draw_eq_mode_toggle(f: &mut Frame, area: Rect, eq_state: &EqState) {
    let is_casual = eq_state.eq_mode == EqMode::Casual;
    let is_enabled = eq_state.eq_enabled;

    let enabled_style = if is_enabled {
        Style::default()
            .fg(Color::Green)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)
    };

    let casual_style = if is_casual {
        Style::default()
            .fg(Color::Green)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::DarkGray)
    };

    let advanced_style = if !is_casual {
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::DarkGray)
    };

    let mode_line = Line::from(vec![
        Span::styled("EQ: ", Style::default().fg(Color::White)),
        Span::styled(if is_enabled { "ON" } else { "OFF" }, enabled_style),
        Span::raw("  │  "),
        Span::styled("Mode: ", Style::default().fg(Color::White)),
        Span::styled(if is_casual { "● " } else { "○ " }, casual_style),
        Span::styled("Casual", casual_style),
        Span::raw("  "),
        Span::styled(if !is_casual { "● " } else { "○ " }, advanced_style),
        Span::styled("Advanced", advanced_style),
        Span::raw("  │  "),
        Span::styled("[T]", Style::default().fg(Color::Yellow)),
        Span::raw(" Toggle EQ  "),
        Span::styled("[M]", Style::default().fg(Color::Yellow)),
        Span::raw(" Mode"),
    ]);

    let paragraph = Paragraph::new(mode_line).block(Block::default().borders(Borders::BOTTOM));
    f.render_widget(paragraph, area);
}

fn draw_eq_controls(
    f: &mut Frame,
    area: Rect,
    eq_state: &EqState,
    eq_focus: EqFocus,
    eq_selected_band: usize,
) {
    let is_casual = eq_state.eq_mode == EqMode::Casual;

    if is_casual {
        // Casual mode: Show preset selector and master gain
        let chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Percentage(50), // Preset
                Constraint::Percentage(50), // Master Gain
            ])
            .split(area);

        // Determine if band panel is focused
        let is_band_focused = eq_focus == EqFocus::BandPanel;
        let band_border_style = if is_band_focused {
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::DarkGray)
        };

        // Preset selector
        let preset_label = format!("{:?}", eq_state.local_preset);
        let preset_paragraph = Paragraph::new(Line::from(vec![
            Span::styled("Preset: ", Style::default().fg(Color::Gray)),
            Span::styled(
                preset_label,
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw("\n"),
            Span::styled("[↑/↓]", Style::default().fg(Color::Yellow)),
            Span::raw(" Change Preset"),
        ]))
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Preset ")
                .border_style(band_border_style),
        );

        f.render_widget(preset_paragraph, chunks[0]);

        // Master gain
        let gain_paragraph = Paragraph::new(Line::from(vec![
            Span::styled("Master: ", Style::default().fg(Color::Gray)),
            Span::styled(
                eq_state.master_gain_label(),
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw("\n"),
            Span::styled("[↑/↓]", Style::default().fg(Color::Yellow)),
            Span::raw(" Adjust Gain"),
        ]))
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Master Gain "),
        );
        f.render_widget(gain_paragraph, chunks[1]);
    } else {
        // Advanced mode: Show filter bands with editable parameters
        let chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Percentage(60), // Filter list
                Constraint::Percentage(40), // Selected filter details
            ])
            .split(area);

        // Determine if band panel is focused
        let is_band_focused = eq_focus == EqFocus::BandPanel;
        let band_border_style = if is_band_focused {
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::DarkGray)
        };

        // Filter band list
        let filter_items: Vec<ListItem> = eq_state
            .local_filters
            .iter()
            .enumerate()
            .map(|(i, filter)| {
                let is_selected = i == eq_selected_band;
                let prefix = if is_selected { "▶ " } else { "  " };
                let style = if is_selected {
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(Color::White)
                };
                let filter_info = format!(
                    "{}Band {}: {:?} @ {}Hz",
                    prefix,
                    i + 1,
                    filter.filter_type,
                    filter.freq as i32
                );
                ListItem::new(filter_info).style(style)
            })
            .collect();

        let filter_list = if filter_items.is_empty() {
            Paragraph::new("No filters. Press [A] to add.")
                .style(Style::default().fg(Color::DarkGray))
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .border_style(band_border_style)
                        .title(" Bands (↑↓ Select) "),
                )
        } else {
            let list = List::new(filter_items).block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(band_border_style)
                    .title(" Bands (↑↓ Select) "),
            );

            let mut list_state = ratatui::widgets::ListState::default();
            list_state.select(Some(eq_selected_band));
            f.render_stateful_widget(list, chunks[0], &mut list_state);
            return draw_filter_details(f, chunks[1], eq_state, eq_selected_band);
        };
        f.render_widget(filter_list, chunks[0]);

        // Empty details panel
        let details = Paragraph::new("Select a band to edit")
            .style(Style::default().fg(Color::DarkGray))
            .block(Block::default().borders(Borders::ALL).title(" Details "));
        f.render_widget(details, chunks[1]);
    }
}

fn draw_filter_details(f: &mut Frame, area: Rect, eq_state: &EqState, eq_selected_band: usize) {
    if eq_state.local_filters.is_empty() {
        let details = Paragraph::new("No band selected")
            .style(Style::default().fg(Color::DarkGray))
            .block(Block::default().borders(Borders::ALL).title(" Details "));
        f.render_widget(details, area);
        return;
    }

    let filter = &eq_state.local_filters[eq_selected_band];
    let params = [
        ("Type", format!("{:?}", filter.filter_type)),
        ("Freq", format!("{} Hz", filter.freq as i32)),
        ("Gain", format!("{:+.1} dB", filter.gain)),
        ("Q", format!("{:.2}", filter.q)),
    ];

    let text: Vec<Line> = params
        .iter()
        .map(|(name, value)| {
            Line::from(vec![
                Span::styled(format!("{}: ", name), Style::default().fg(Color::Gray)),
                Span::styled(value.clone(), Style::default().fg(Color::White)),
            ])
        })
        .collect();

    let paragraph =
        Paragraph::new(text).block(Block::default().borders(Borders::ALL).title(" Details "));
    f.render_widget(paragraph, area);
}

fn draw_eq_graph(
    f: &mut Frame,
    area: Rect,
    eq_state: &EqState,
    audio_state: &AudioState,
    eq_focus: EqFocus,
    eq_selected_band: usize,
) {
    const RESPONSE_POINT_COUNT: usize = 256;
    const MIN_GAIN_DB: f64 = -18.0;
    const MAX_GAIN_DB: f64 = 18.0;

    // Create a temporary Equalizer to compute the response curve
    let sample_rate = audio_state
        .metadata
        .as_ref()
        .map_or(44100, |m| m.sample_rate);
    let mut eq = Equalizer::new(sample_rate, eq_state.local_num_channels);
    eq.filters = eq_state.local_filters.clone();
    eq.master_gain = (10.0f32).powf(eq_state.local_master_gain / 20.0); // Convert dB to linear
    eq.parameters_changed();

    let data = eq.get_response_curve(RESPONSE_POINT_COUNT);

    // Transform to log scale for x-axis (frequency)
    let data_points: Vec<(f64, f64)> = data
        .iter()
        .map(|(freq, db)| ((*freq as f64).log10(), *db as f64))
        .collect();

    let filter_curves: Vec<Vec<(f64, f64)>> = eq_state
        .local_filters
        .iter()
        .map(|filter| {
            let mut curve_points = Vec::with_capacity(RESPONSE_POINT_COUNT);
            let start_freq: f32 = 20.0;
            let end_freq: f32 = 20000.0;
            let log_start = start_freq.ln();
            let log_end = end_freq.ln();
            let step = (log_end - log_start) / (RESPONSE_POINT_COUNT as f32 - 1.0);

            for i in 0..RESPONSE_POINT_COUNT {
                let log_f = log_start + step * (i as f32);
                let frequency = log_f.exp();
                let db = filter.magnitude_db(frequency, sample_rate as f32);
                curve_points.push((frequency.log10() as f64, db as f64));
            }

            curve_points
        })
        .collect();

    let filter_points: Vec<(f64, f64)> = eq_state
        .local_filters
        .iter()
        .map(|filter| {
            let mut total_db = eq_state.local_master_gain;
            for flt in &eq_state.local_filters {
                total_db += flt.magnitude_db(filter.freq, sample_rate as f32);
            }
            ((filter.freq as f64).log10(), total_db as f64)
        })
        .collect();

    let is_focused = eq_focus == EqFocus::CurvePanel;
    let border_style = if is_focused {
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::DarkGray)
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(border_style)
        .title(format!(
            " Frequency Response (↑↓ Gain) • Master {} ",
            eq_state.master_gain_label()
        ));
    let inner = block.inner(area);
    f.render_widget(block, area);

    if inner.width < 8 || inner.height < 3 {
        return;
    }

    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(4), Constraint::Min(4)])
        .split(inner);
    let graph_rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(2), Constraint::Length(1)])
        .split(columns[1]);
    let y_axis_area = Rect::new(
        columns[0].x,
        columns[0].y,
        columns[0].width,
        graph_rows[0].height,
    );
    let plot_area = graph_rows[0];
    let x_axis_area = graph_rows[1];

    draw_y_axis_labels(f, y_axis_area);
    draw_x_axis_labels(f, x_axis_area);

    let min_frequency_log = 20.0_f64.log10();
    let max_frequency_log = 20000.0_f64.log10();
    let canvas = Canvas::default()
        .marker(symbols::Marker::Braille)
        .x_bounds([min_frequency_log, max_frequency_log])
        .y_bounds([MIN_GAIN_DB, MAX_GAIN_DB])
        .paint(|ctx| {
            // Log-frequency and gain grid.
            for frequency in [
                20.0_f64, 50.0, 100.0, 200.0, 500.0, 1000.0, 2000.0, 5000.0, 10000.0, 20000.0,
            ] {
                let x = frequency.log10();
                ctx.draw(&CanvasLine::new(
                    x,
                    MIN_GAIN_DB,
                    x,
                    MAX_GAIN_DB,
                    Color::DarkGray,
                ));
            }
            for gain in [-12.0_f64, -6.0, 6.0, 12.0] {
                ctx.draw(&CanvasLine::new(
                    min_frequency_log,
                    gain,
                    max_frequency_log,
                    gain,
                    Color::DarkGray,
                ));
            }
            ctx.draw(&CanvasLine::new(
                min_frequency_log,
                0.0,
                max_frequency_log,
                0.0,
                Color::Gray,
            ));
            ctx.layer();

            // Draw each band's response so its contribution to the master curve is visible.
            for (index, curve) in filter_curves.iter().enumerate() {
                let color = if index == eq_selected_band {
                    Color::Magenta
                } else {
                    Color::Blue
                };
                draw_canvas_curve(ctx, curve, color);
            }
            ctx.layer();

            draw_canvas_curve(ctx, &data_points, Color::Cyan);
            ctx.layer();

            // Cross-shaped handles stay visible at each band's center frequency.
            for (index, &(x, y)) in filter_points.iter().enumerate() {
                let (color, half_width, half_height) = if index == eq_selected_band {
                    (Color::Yellow, 0.025, 1.0)
                } else {
                    (Color::White, 0.015, 0.7)
                };
                ctx.draw(&CanvasLine::new(
                    x - half_width,
                    y,
                    x + half_width,
                    y,
                    color,
                ));
                ctx.draw(&CanvasLine::new(
                    x,
                    y - half_height,
                    x,
                    y + half_height,
                    color,
                ));
                ctx.draw(&Points::new(&[(x, y)], color));
            }
        });

    f.render_widget(canvas, plot_area);
}

fn draw_canvas_curve(ctx: &mut CanvasContext<'_>, points: &[(f64, f64)], color: Color) {
    for pair in points.windows(2) {
        let [(x1, y1), (x2, y2)] = pair else {
            continue;
        };
        if x1.is_finite() && y1.is_finite() && x2.is_finite() && y2.is_finite() {
            ctx.draw(&CanvasLine::new(*x1, *y1, *x2, *y2, color));
        }
    }
}

fn draw_y_axis_labels(f: &mut Frame, area: Rect) {
    for (y, label) in [
        (area.top(), "+18"),
        (area.top() + area.height / 2, "0"),
        (area.bottom().saturating_sub(1), "-18"),
    ] {
        f.render_widget(
            Paragraph::new(label)
                .alignment(Alignment::Right)
                .style(Style::default().fg(Color::Gray)),
            Rect::new(area.x, y, area.width.saturating_sub(1), 1),
        );
    }
}

fn draw_x_axis_labels(f: &mut Frame, area: Rect) {
    let labels = [
        (0.0_f64, "20"),
        (1.0 / 3.0, "200"),
        (2.0 / 3.0, "2k"),
        (1.0, "20k"),
    ];

    for (position, label) in labels {
        let label_width = label.chars().count() as u16;
        let center = area.x + ((area.width.saturating_sub(1) as f64 * position).round() as u16);
        let x = center
            .saturating_sub(label_width / 2)
            .clamp(area.x, area.right().saturating_sub(label_width));
        f.render_widget(
            Paragraph::new(label).style(Style::default().fg(Color::Gray)),
            Rect::new(x, area.y, label_width, 1),
        );
    }
}

fn draw_filter_band_config_modal(
    f: &mut Frame,
    area: Rect,
    eq_state: &EqState,
    config: &BandFilterConfig,
    locked_in: bool,
) {
    let filter = match eq_state.local_filters.get(config.selected_band) {
        Some(f) => f,
        None => return,
    };

    let title = format!(" Band {} Configuration ", config.selected_band + 1);
    let selected_param = config.selected_param;

    let params: Vec<(&str, String, Color)> = vec![
        ("Type", format!("{}", filter.filter_type), Color::Cyan),
        (
            "Frequency",
            format!("{} Hz", filter.freq as i32),
            Color::Green,
        ),
        (
            "Gain",
            format!("{:+.1} dB", filter.gain),
            if filter.gain >= 0.0 {
                Color::Green
            } else {
                Color::Red
            },
        ),
        ("Q Factor", format!("{:.3}", filter.q), Color::Yellow),
        ("Order", filter.order.to_string(), Color::Magenta),
    ];

    let mut text: Vec<Line> = params
        .iter()
        .enumerate()
        .map(|(i, (label, value, color))| {
            let is_selected = i == selected_param;
            let is_locked = is_selected && locked_in;
            let prefix = if is_locked {
                "⏺ "
            } else if is_selected {
                "▶ "
            } else {
                "  "
            };
            let label_style = if is_locked {
                Style::default()
                    .fg(Color::White)
                    .bg(Color::DarkGray)
                    .add_modifier(Modifier::BOLD)
            } else if is_selected {
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::Gray)
            };
            let value_style = if is_locked {
                Style::default()
                    .fg(*color)
                    .bg(Color::DarkGray)
                    .add_modifier(Modifier::BOLD)
            } else if is_selected {
                Style::default().fg(*color).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::DarkGray)
            };
            Line::from(vec![
                Span::styled(
                    format!("{}{:<12}", prefix, format!("{}:", label)),
                    label_style,
                ),
                Span::styled(value.clone(), value_style),
            ])
        })
        .collect();

    // Add a blank separator line and control hints
    text.push(Line::from(""));

    let hints = if locked_in {
        Line::from(vec![
            Span::styled("[←/→]", Style::default().fg(Color::Yellow)),
            Span::raw(" Adjust  "),
            Span::styled("[Enter]", Style::default().fg(Color::Yellow)),
            Span::raw(" Unlock  "),
            Span::styled("[Esc]", Style::default().fg(Color::Yellow)),
            Span::raw(" Close"),
        ])
    } else {
        Line::from(vec![
            Span::styled("[↑/↓]", Style::default().fg(Color::Yellow)),
            Span::raw(" Navigate  "),
            Span::styled("[Enter]", Style::default().fg(Color::Yellow)),
            Span::raw(" Edit  "),
            Span::styled("[Esc]", Style::default().fg(Color::Yellow)),
            Span::raw(" Close"),
        ])
    };
    text.push(hints);

    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Yellow));

    let paragraph = Paragraph::new(text).block(block);
    f.render_widget(paragraph, area);
}

#[cfg(test)]
mod tests {
    use ratatui::{Terminal, backend::TestBackend};

    use super::*;

    #[test]
    fn advanced_eq_renders_canvas_and_master_gain() {
        let mut eq_state = EqState::new();
        eq_state.eq_mode = EqMode::Advanced;
        eq_state.local_master_gain = 3.5;
        let audio_state = AudioState::new();
        let mut terminal = Terminal::new(TestBackend::new(100, 24)).unwrap();

        terminal
            .draw(|frame| {
                draw_eq_panel(
                    frame,
                    frame.area(),
                    &eq_state,
                    &audio_state,
                    EqFocus::CurvePanel,
                    0,
                );
            })
            .unwrap();

        let buffer = terminal.backend().buffer();
        let rendered: String = buffer.content().iter().map(|cell| cell.symbol()).collect();

        assert!(rendered.contains("Master +3.5 dB"));
        assert!(buffer.content().iter().any(|cell| {
            cell.symbol()
                .chars()
                .any(|symbol| ('\u{2800}'..='\u{28ff}').contains(&symbol))
        }));
    }
}
