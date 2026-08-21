use anyhow::Result;
use ratatui::{Frame, layout::Rect};

use crate::{
    routes::{
        browser::BrowserRoute, eq::EqualizerRoute, log::LogRoute, playback::PlaybackRoute,
        queue::QueueRoute, settings::SettingsRoute,
    },
    state::AppState,
};
use audido_core::modules::core::CoreHandle;
use ratatui::crossterm::event::KeyCode;

/// Trait that all routes must implement
/// This enables dynamic dispatch and polymorphic behavior
pub trait RouteHandler: std::fmt::Debug {
    /// Render this route's UI
    fn render(&mut self, frame: &mut Frame, area: Rect, state: &AppState);

    /// Handle keyboard input for this route
    /// Returns Ok(RouteAction) to indicate what should happen next
    fn handle_input(
        &mut self,
        key: KeyCode,
        state: &mut AppState,
        handle: &CoreHandle,
    ) -> anyhow::Result<RouteAction>;

    /// Get the display name for breadcrumbs/navigation
    fn name(&self) -> &str;

    /// Optional: Called when entering this route
    fn on_enter(&mut self, _state: &mut AppState, _handle: &CoreHandle) -> Result<()> {
        Ok(())
    }

    /// Optional: Called when leaving this route
    fn on_exit(&mut self, _state: &mut AppState, _handle: &CoreHandle) -> Result<()> {
        Ok(())
    }

    /// Optional: Check if this route can be exited (for validation)
    fn can_exit(&self, _state: &AppState) -> bool {
        true
    }

    #[allow(dead_code)]
    fn help_items(&self, _state: &AppState) -> Vec<(&str, &str)> {
        vec![("Tab", "Switch Tab"), ("Q", "Quit")]
    }

    fn intercept_global_key(
        &mut self,
        #[allow(unused_variables)] key: KeyCode,
        #[allow(unused_variables)] state: &mut AppState,
        #[allow(unused_variables)] handle: &CoreHandle,
    ) -> InterceptKeyResult {
        InterceptKeyResult::Ignored
    }
}

/// Actions that can be returned from route handlers
#[derive(Debug)]
pub enum RouteAction {
    /// Do nothing, stay on current route
    None,
    /// Go back to previous route
    #[allow(dead_code)]
    Pop,
    /// Navigate to a new route
    Push(Box<dyn RouteHandler>),
    /// Replace current route with a new one
    Replace(Box<dyn RouteHandler>),
    /// Clear stack and navigate to route
    #[allow(dead_code)]
    Reset(Box<dyn RouteHandler>),
    /// Quit the application
    #[allow(dead_code)]
    Quit,
}

#[derive(Debug)]
pub enum InterceptKeyResult {
    Handled,
    Ignored,
    #[allow(dead_code)]
    HandledAndNavigate(RouteAction),
}

/// Router manages the navigation stack
pub struct Router {
    /// Stack of route handlers, last element is current route
    stack: Vec<Box<dyn RouteHandler>>,
}

impl Router {
    pub fn new(initial_route: Box<dyn RouteHandler>) -> Self {
        Self {
            stack: vec![initial_route],
        }
    }

    /// Get current route (top of stack)
    pub fn current(&self) -> &dyn RouteHandler {
        self.stack
            .last()
            .expect("Stack should never be empty")
            .as_ref()
    }

    /// Get mutable reference to current route
    pub fn current_mut(&mut self) -> &mut Box<dyn RouteHandler> {
        self.stack.last_mut().expect("Stack should never be empty")
    }

    /// Execute a route action
    pub fn execute_action(
        &mut self,
        action: RouteAction,
        state: &mut AppState,
        handle: &CoreHandle,
    ) -> Result<bool> {
        match action {
            RouteAction::None => Ok(false),
            RouteAction::Pop => {
                self.pop(state, handle)?;
                Ok(false)
            }
            RouteAction::Push(route) => {
                self.push(route, state, handle)?;
                Ok(false)
            }
            RouteAction::Replace(route) => {
                self.replace(route, state, handle)?;
                Ok(false)
            }
            RouteAction::Reset(route) => {
                self.reset_to(route, state, handle)?;
                Ok(false)
            }
            RouteAction::Quit => Ok(true),
        }
    }

    /// Navigate to a new route (push onto stack)
    pub fn push(
        &mut self,
        mut route: Box<dyn RouteHandler>,
        state: &mut AppState,
        handle: &CoreHandle,
    ) -> Result<()> {
        route.on_enter(state, handle)?;
        self.stack.push(route);
        Ok(())
    }

    /// Go back (pop from stack)
    pub fn pop(
        &mut self,
        state: &mut AppState,
        handle: &CoreHandle,
    ) -> Result<Option<Box<dyn RouteHandler>>> {
        // Keep at least one route in the stack
        if self.stack.len() > 1 {
            if let Some(current) = self.stack.last()
                && !current.can_exit(state)
            {
                return Ok(None);
            }

            if let Some(mut route) = self.stack.pop() {
                route.on_exit(state, handle)?;
                return Ok(Some(route));
            }
        }
        Ok(None)
    }

    /// Replace current route (useful for tab switching)
    pub fn replace(
        &mut self,
        mut new_route: Box<dyn RouteHandler>,
        state: &mut AppState,
        handle: &CoreHandle,
    ) -> Result<()> {
        if let Some(mut old_route) = self.stack.pop() {
            old_route.on_exit(state, handle)?;
        }
        new_route.on_enter(state, handle)?;
        self.stack.push(new_route);
        Ok(())
    }

    /// Clear stack and navigate to route
    pub fn reset_to(
        &mut self,
        mut route: Box<dyn RouteHandler>,
        state: &mut AppState,
        handle: &CoreHandle,
    ) -> Result<()> {
        // Exit all routes
        while let Some(mut old_route) = self.stack.pop() {
            old_route.on_exit(state, handle)?;
        }
        route.on_enter(state, handle)?;
        self.stack.push(route);
        Ok(())
    }

    /// Get the depth of navigation
    pub fn depth(&self) -> usize {
        self.stack.len()
    }
}

/// Get a route handler for a given tab name
pub fn route_for_name(name: &str) -> Box<dyn RouteHandler> {
    match name {
        "Playback" => Box::new(PlaybackRoute),
        "Queue" => Box::new(QueueRoute),
        "Browser" => Box::new(BrowserRoute::new()),
        "Settings" => Box::new(SettingsRoute),
        "Log" => Box::new(LogRoute::new()),
        "Equalizer" => Box::new(EqualizerRoute::default()),
        _ => Box::new(PlaybackRoute),
    }
}

static TAB_NAMES: [&str; 5] = ["Playback", "Queue", "Browser", "Settings", "Log"];

/// Get all main tab names in order
pub fn tab_names() -> &'static [&'static str] {
    &TAB_NAMES
}

/// get the next tab name from the current_tab
pub fn get_next_tab(current_tab: &str) -> Option<&'static str> {
    let index = TAB_NAMES.iter().position(|&x| x == current_tab);
    if let Some(index) = index {
        return Some(TAB_NAMES[(index + 1) % TAB_NAMES.len()]);
    }
    None
}
