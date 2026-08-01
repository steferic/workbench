//! A queue of work you want one agent to get through, one item at a time.
//!
//! This is workbench's own state, unlike the agent's task list — which is a
//! read-only mirror of something the agent maintains only when it feels like
//! it. Items here are yours: they persist, they stay until you clear them,
//! and the dispatcher (see `app::todo_dispatch`) feeds them to the agent as
//! its turns end.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Where an item is in its life.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum TodoState {
    /// Waiting its turn.
    #[default]
    Pending,
    /// Handed to the agent; waiting for the turn to end.
    Running,
    /// The agent's turn ended after this was sent.
    Done,
}

/// One thing for the agent to do.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueuedTodo {
    pub id: Uuid,
    pub text: String,
    #[serde(default)]
    pub state: TodoState,
    pub created_at: DateTime<Utc>,
    /// When it was sent to the agent, if it has been.
    #[serde(default)]
    pub sent_at: Option<DateTime<Utc>>,
}

impl QueuedTodo {
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            id: Uuid::new_v4(),
            text: text.into(),
            state: TodoState::Pending,
            created_at: Utc::now(),
            sent_at: None,
        }
    }
}

/// One agent's queue: an ordered list, at most one item in flight.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TodoQueue {
    #[serde(default)]
    pub items: Vec<QueuedTodo>,
    /// Paused queues hold their items instead of dispatching them.
    #[serde(default)]
    pub paused: bool,
}

impl TodoQueue {
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn add(&mut self, text: impl Into<String>) -> Uuid {
        let todo = QueuedTodo::new(text);
        let id = todo.id;
        self.items.push(todo);
        id
    }

    pub fn remove(&mut self, id: Uuid) {
        self.items.retain(|item| item.id != id);
    }

    pub fn get_mut(&mut self, id: Uuid) -> Option<&mut QueuedTodo> {
        self.items.iter_mut().find(|item| item.id == id)
    }

    /// Move an item one place earlier or later, so you can reorder work
    /// without retyping it.
    pub fn shift(&mut self, id: Uuid, delta: isize) {
        let Some(from) = self.items.iter().position(|item| item.id == id) else {
            return;
        };
        let to = from as isize + delta;
        if to < 0 || to >= self.items.len() as isize {
            return;
        }
        let item = self.items.remove(from);
        self.items.insert(to as usize, item);
    }

    /// The item currently with the agent.
    pub fn running(&self) -> Option<&QueuedTodo> {
        self.items.iter().find(|i| i.state == TodoState::Running)
    }

    /// The item that would go next — `None` while one is still in flight,
    /// since the point of a queue is one thing at a time.
    pub fn next_pending(&self) -> Option<&QueuedTodo> {
        if self.running().is_some() {
            return None;
        }
        self.items.iter().find(|i| i.state == TodoState::Pending)
    }

    pub fn pending_count(&self) -> usize {
        self.items
            .iter()
            .filter(|i| i.state == TodoState::Pending)
            .count()
    }

    /// Mark an item as handed over.
    pub fn mark_running(&mut self, id: Uuid) {
        if let Some(item) = self.get_mut(id) {
            item.state = TodoState::Running;
            item.sent_at = Some(Utc::now());
        }
    }

    /// The agent's turn ended, so whatever was in flight is finished.
    pub fn finish_running(&mut self) -> Option<Uuid> {
        let item = self
            .items
            .iter_mut()
            .find(|i| i.state == TodoState::Running)?;
        item.state = TodoState::Done;
        Some(item.id)
    }

    /// Put a running item back at the head of the queue — used when a session
    /// dies mid-item, so the work is not silently lost.
    pub fn requeue_running(&mut self) {
        for item in self.items.iter_mut() {
            if item.state == TodoState::Running {
                item.state = TodoState::Pending;
                item.sent_at = None;
            }
        }
    }

    pub fn clear_completed(&mut self) {
        self.items.retain(|item| item.state != TodoState::Done);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn queue_of(texts: &[&str]) -> TodoQueue {
        let mut queue = TodoQueue::default();
        for text in texts {
            queue.add(*text);
        }
        queue
    }

    #[test]
    fn items_are_handed_over_one_at_a_time_in_order() {
        let mut queue = queue_of(&["first", "second"]);

        let next = queue.next_pending().unwrap().id;
        assert_eq!(queue.items[0].id, next);
        queue.mark_running(next);

        // Nothing else goes out while one is with the agent.
        assert!(queue.next_pending().is_none());
        assert_eq!(queue.pending_count(), 1);

        queue.finish_running();
        assert_eq!(queue.items[0].state, TodoState::Done);
        assert_eq!(queue.next_pending().unwrap().text, "second");
    }

    #[test]
    fn a_session_that_dies_mid_item_gives_the_work_back() {
        let mut queue = queue_of(&["migrate the schema"]);
        let id = queue.next_pending().unwrap().id;
        queue.mark_running(id);

        queue.requeue_running();

        assert_eq!(queue.items[0].state, TodoState::Pending);
        assert!(queue.items[0].sent_at.is_none());
        assert_eq!(queue.next_pending().unwrap().id, id, "it goes out again");
    }

    #[test]
    fn reordering_moves_one_place_and_stops_at_the_ends() {
        let mut queue = queue_of(&["a", "b", "c"]);
        let (a, b, c) = (queue.items[0].id, queue.items[1].id, queue.items[2].id);

        queue.shift(c, -1);
        assert_eq!(order(&queue), vec![a, c, b]);

        // Already first / already last: no move, no panic.
        queue.shift(a, -1);
        queue.shift(b, 1);
        assert_eq!(order(&queue), vec![a, c, b]);
    }

    #[test]
    fn clearing_completed_leaves_the_work_that_remains() {
        let mut queue = queue_of(&["done one", "still to do"]);
        let first = queue.items[0].id;
        queue.mark_running(first);
        queue.finish_running();

        queue.clear_completed();

        assert_eq!(queue.items.len(), 1);
        assert_eq!(queue.items[0].text, "still to do");
    }

    fn order(queue: &TodoQueue) -> Vec<Uuid> {
        queue.items.iter().map(|i| i.id).collect()
    }
}
