//! Glue between the egui thread and the engine's async world. The UI never blocks: it sends a command
//! and gets a [`UiMsg`] back over a channel, and every send asks egui to repaint.

use std::sync::Arc;
use std::sync::mpsc::Sender;

use serde_json::Value;
use tokio::runtime::Runtime;
use workcraft_agents::Engine;
use workcraft_control::CommandBus;
use workcraft_core::{Event, Result};

/// What the UI thread receives.
pub enum UiMsg {
    Event(Box<Event>),
    /// Result of a command the UI started, with a tag saying which one.
    Done(&'static str, Result<Value>),
}

#[derive(Clone)]
pub struct Bridge {
    pub rt: Arc<Runtime>,
    pub engine: Engine,
    tx: Sender<UiMsg>,
    ctx: eframe::egui::Context,
}

impl Bridge {
    pub fn new(
        rt: Arc<Runtime>,
        engine: Engine,
        tx: Sender<UiMsg>,
        ctx: eframe::egui::Context,
    ) -> Self {
        let b = Self {
            rt,
            engine,
            tx,
            ctx,
        };
        b.forward_events();
        b
    }

    fn forward_events(&self) {
        let mut rx = self.engine.subscribe_events();
        let (tx, ctx) = (self.tx.clone(), self.ctx.clone());
        self.rt.spawn(async move {
            loop {
                match rx.recv().await {
                    Ok(ev) => {
                        if tx.send(UiMsg::Event(Box::new(ev))).is_err() {
                            break;
                        }
                        ctx.request_repaint();
                    }
                    // A burst of text deltas can overrun the buffer; the next message-added event resyncs.
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
        });
    }

    /// Run a command id in the background; the answer arrives as `UiMsg::Done(tag, result)`.
    pub fn run(&self, tag: &'static str, command: &'static str, params: Value) {
        let (engine, tx, ctx) = (self.engine.clone(), self.tx.clone(), self.ctx.clone());
        self.rt.spawn(async move {
            let r = engine.execute(command, params).await;
            let _ = tx.send(UiMsg::Done(tag, r));
            ctx.request_repaint();
        });
    }
}
