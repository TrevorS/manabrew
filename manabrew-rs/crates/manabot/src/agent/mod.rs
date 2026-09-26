//! Decision strategies for a bot. The lifecycle in `BotState` is fixed; the
//! AI plugged into it is not.
//!
//! To add a new agent: define a struct implementing [`BotAgent`], add a
//! variant to [`AgentKind`], and wire it in [`AgentKind::build`]. The room
//! picks which agent to spawn via the `agent` field of the bot config.
//!
//! A hosted room may also answer its own bot seats in process rather than
//! over the relay, and it builds that responder from the same configured
//! agent (`self-hosted-node`'s `Config::bot_agent`). Anything that chooses an
//! agent has to set that one field; a second, separate choice would win over
//! the seat's own, because the relay's answer arrives later and is discarded
//! as a duplicate -- with no error, and a legal-looking game.

use manabrew_agent_interface::agent_impl::Responder;
use manabrew_agent_interface::game_view_dto::GameViewDto;
use manabrew_agent_interface::prompt::{
    AgentMessage, AgentPrompt, ChooseActionOutput, ClientToServerMessage, PromptOutput,
};
use serde::{Deserialize, Serialize};

pub mod simple_ai;
pub use simple_ai::SimpleAi;

pub trait BotAgent: Send {
    fn observe(&mut self, _view: GameViewDto) {}
    fn decide(&mut self, prompt: AgentPrompt) -> Option<PromptOutput>;
    fn fork(&self) -> Option<Box<dyn BotAgent + Send>> {
        None
    }
}

/// Wire-level selector for which built-in agent the bot should use.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum AgentKind {
    #[default]
    Simple,
}

impl AgentKind {
    pub fn build(self) -> Box<dyn BotAgent + Send> {
        match self {
            AgentKind::Simple => Box::<SimpleAi>::default(),
        }
    }
}

pub struct BotResponder {
    agent: Box<dyn BotAgent + Send>,
}

impl BotResponder {
    pub fn new(agent: Box<dyn BotAgent + Send>) -> Self {
        Self { agent }
    }

    pub fn fork(&self) -> Option<Self> {
        self.agent.fork().map(Self::new)
    }
}

impl Default for BotResponder {
    fn default() -> Self {
        Self::new(AgentKind::default().build())
    }
}

impl BotResponder {
    /// Feed the seat's latest view to the agent, as a state envelope would.
    pub fn observe(&mut self, view: GameViewDto) {
        self.agent.observe(view);
    }
}

impl Responder for BotResponder {
    fn respond(&mut self, prompt: AgentPrompt) -> ClientToServerMessage {
        let prompt_id = prompt.prompt_id;
        let action = self
            .agent
            .decide(prompt)
            .unwrap_or(PromptOutput::ChooseAction(ChooseActionOutput::Pass {
                until: None,
                exhaust_stack: false,
            }));
        ClientToServerMessage::Response { prompt_id, action }
    }

    fn present(&mut self, message: &AgentMessage) {
        if let AgentMessage::State(update) = message {
            self.observe(update.game_view.clone());
        }
    }
}
