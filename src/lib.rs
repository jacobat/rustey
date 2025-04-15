use log::info;
use ratatui::prelude::*;
use simplelog::{Config, LevelFilter, WriteLogger};
use std::any::Any;
use std::fs::File;
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

pub type Sender<T> = mpsc::Sender<T>;

pub type Subscriptions<M> = Vec<Box<dyn Subscription<M>>>;

pub trait Subscription<T>: DynEq + Send + Sync {
    fn run(&self, sender: mpsc::Sender<T>, alive: QuitFlag);
}

pub trait Command<T>: Send + Sync {
    fn run(&self, sender: mpsc::Sender<T>);
}
pub trait TearApp<T, M> {
    fn init(&self) -> (T, Cmd<M>);
    fn update(&self, model: &mut T, msg: M, quit_program: &QuitFlag) -> Cmd<M>;
    fn subscriptions(&self, model: &T) -> Vec<Box<dyn Subscription<M>>>;
    fn view(&self, frame: &mut Frame, model: &T);
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

    pub fn run(&mut self, sender: mpsc::Sender<T>) {
        let sub = self.sub.clone();
        let halt_flag = self.halt_flag.clone();
        self.thread = Some(thread::spawn(move || sub.run(sender, halt_flag)));
    }

    pub fn stop(&mut self) {
        self.halt_flag.raise();
    }
}

fn handle<M>(cmd: Cmd<M>, sender: mpsc::Sender<M>)
where
    M: 'static + Send + Sync,
{
    if let Some(c) = cmd {
        thread::spawn(move || {
            c.run(sender);
        });
    }
}

pub fn run<T, M>(app: &dyn TearApp<T, M>) -> io::Result<()>
where
    M: 'static + Send + Sync,
    T: 'static + Send + Sync,
{
    let _ = WriteLogger::init(
        LevelFilter::Info,
        Config::default(),
        File::create("my_rust_bin.log").unwrap(),
    );

    let mut terminal = ratatui::init();

    let (mut model, cmd) = app.init();
    let quit_program = QuitFlag::new();
    let (sender, receiver) = std::sync::mpsc::channel::<M>();
    let initial_subscriptions = app.subscriptions(&model);

    let subs: Vec<SubRec<M>> = initial_subscriptions.into_iter().map(SubRec::new).collect();

    let mut subs: Vec<SubRec<M>> = subs
        .into_iter()
        .map(|mut sub| {
            sub.run(sender.clone());
            sub
        })
        .collect();

    handle(cmd, sender.clone());

    loop {
        info!("Looping");
        terminal.draw(|f| app.view(f, &model))?;
        let msg = receiver.recv().unwrap();
        let cmd = app.update(&mut model, msg, &quit_program);
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
            s.run(sender.clone());
        });
        subs.append(&mut new_subs);
        info!("Subscriptions: {:?}", subs.len());

        handle(cmd, sender.clone());

        if quit_program.raised() {
            break;
        }
    }

    ratatui::restore();
    Ok(())
}
