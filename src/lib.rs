use ratatui::crossterm::event::{self, Event, poll};
use ratatui::prelude::*;
use std::any::Any;
use std::io;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
    mpsc,
};
use std::thread;

impl<T> PartialEq for dyn Subscription<T> {
    fn eq(&self, other: &Self) -> bool {
        self.equals_a(other)
    }
}

pub struct QuitFlag {
    quit: Arc<AtomicBool>,
}

impl QuitFlag {
    fn new() -> Self {
        Self {
            quit: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn raise(&self) {
        self.quit.store(true, Ordering::Relaxed);
    }

    pub fn raised(&self) -> bool {
        self.quit.load(Ordering::Relaxed)
    }

    fn clone(&self) -> QuitFlag {
        QuitFlag {
            quit: self.quit.clone(),
        }
    }
}

pub struct Sender<T: 'static> {
    sender: mpsc::Sender<Message<T>>,
}

impl<T> Sender<T> {
    fn new(sender: mpsc::Sender<Message<T>>) -> Self {
        Self { sender }
    }

    pub fn send(&self, msg: T) {
        let msg = Message::UserEvent(msg);
        let _ = self.sender.send(msg);
    }

    fn dup(&self) -> Self {
        Sender {
            sender: self.sender.clone(),
        }
    }
}

pub enum Message<T: 'static> {
    UserEvent(T),
    CrosstermEvent(Event),
}

pub type Subscriptions<M> = Vec<Box<dyn Subscription<M>>>;

pub trait Subscription<T>: DynEq + Send + Sync {
    fn run(&self, sender: Sender<T>, alive: QuitFlag);
}

pub trait Command<T>: Send + Sync {
    fn run(&self, sender: Sender<T>);
}
pub trait RusteyApp<T, M> {
    fn init(&self) -> (T, Cmd<M>);
    fn map_event(&self, model: &T, event: Event) -> Option<M>;
    fn update(&self, model: &mut T, msg: M, quit_program: &QuitFlag) -> Cmd<M>;
    fn subscriptions(&self, model: &T) -> Vec<Box<dyn Subscription<M>>>;
    fn view(&self, frame: &mut Frame, model: &mut T);
}

pub type Cmd<T> = Option<Box<dyn Command<T>>>;

pub trait DynEq {
    // An &Any can be cast to a reference to a concrete type.
    fn as_any(&self) -> &dyn Any;

    // Perform the test.
    fn equals_a(&self, _: &dyn DynEq) -> bool;
}

// Implement DynEq for all 'static types implementing PartialEq.
impl<S: 'static + PartialEq> DynEq for S {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn equals_a(&self, other: &dyn DynEq) -> bool {
        // Do a type-safe casting. If the types are different,
        // return false, otherwise test the values for equality.
        other.as_any().downcast_ref::<S>() == Some(self)
    }
}

pub struct SubRec<T> {
    sub: Arc<Box<dyn Subscription<T>>>,
    halt_flag: QuitFlag,
    thread: Option<thread::JoinHandle<()>>,
}

impl<T: 'static> PartialEq for SubRec<T> {
    fn eq(&self, other: &Self) -> bool {
        self.sub.equals_a(&other.sub)
    }
}

impl<T: Send + 'static> SubRec<T> {
    pub fn new(sub: Box<dyn Subscription<T>>) -> SubRec<T> {
        Self {
            sub: Arc::new(sub),
            halt_flag: QuitFlag::new(),
            thread: None,
        }
    }

    pub fn run(&mut self, sender: Sender<T>) {
        let sub = self.sub.clone();
        let halt_flag = self.halt_flag.clone();
        self.thread = Some(thread::spawn(move || sub.run(sender, halt_flag)));
    }

    pub fn stop(&mut self) {
        self.halt_flag.raise();
    }
}

fn handle<M>(cmd: Cmd<M>, sender: Sender<M>)
where
    M: 'static + Send + Sync,
{
    if let Some(c) = cmd {
        thread::spawn(move || {
            c.run(sender);
        });
    }
}

fn start_event_thread<M: Send>(
    sender: std::sync::mpsc::Sender<Message<M>>,
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        loop {
            let poll = poll(std::time::Duration::from_millis(100));
            if let Ok(true) = poll {
                let event = event::read().unwrap();
                let _ = sender.send(Message::CrosstermEvent(event));
            }
        }
    })
}

pub fn run<T, M>(app: &dyn RusteyApp<T, M>) -> io::Result<()>
where
    M: 'static + Send + Sync,
{
    let mut terminal = ratatui::init();

    let (mut model, mut cmd) = app.init();
    let quit_program = QuitFlag::new();
    let (sender, receiver) = std::sync::mpsc::channel::<Message<M>>();
    let user_sender = Sender::new(sender.clone());
    let initial_subscriptions = app.subscriptions(&model);
    let _event_reader = start_event_thread(sender);

    let subs: Vec<SubRec<M>> = initial_subscriptions.into_iter().map(SubRec::new).collect();

    let mut subs: Vec<SubRec<M>> = subs
        .into_iter()
        .map(|mut sub| {
            sub.run(user_sender.dup());
            sub
        })
        .collect();

    loop {
        handle(cmd, user_sender.dup());
        terminal.draw(|f| app.view(f, &mut model))?;
        let msg = receiver.recv().unwrap();
        let msg = match msg {
            Message::CrosstermEvent(event) => app.map_event(&model, event),
            Message::UserEvent(msg) => Some(msg),
        };

        cmd = if let Some(msg) = msg {
            app.update(&mut model, msg, &quit_program)
        } else {
            None
        };

        let new_subscriptions = app.subscriptions(&model);
        let mut new_subs: Vec<SubRec<M>> = new_subscriptions.into_iter().map(SubRec::new).collect();
        subs.retain_mut(|sub| {
            let pos = new_subs.iter().position(|new_sub| sub == new_sub);
            if let Some(pos1) = pos {
                new_subs.swap_remove(pos1);
                true
            } else {
                sub.stop();
                false
            }
        });
        new_subs.iter_mut().for_each(|s| {
            s.run(user_sender.dup());
        });
        subs.append(&mut new_subs);

        if quit_program.raised() {
            break;
        }
    }

    ratatui::restore();
    Ok(())
}
