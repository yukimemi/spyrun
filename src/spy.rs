// =============================================================================
// File        : spy.rs
// Author      : yukimemi
// Last Change : 2023/10/11 02:51:46.
// =============================================================================

use std::{
    path::Path,
    sync::mpsc,
    thread::{self, JoinHandle},
    time::Duration,
};

use anyhow::Result;
use log_derive::logfn;
use notify::{
    event::{EventAttributes, ModifyKind},
    recommended_watcher, Config, Event, EventKind, PollWatcher, RecommendedWatcher, Watcher,
};
use rand::Rng;
use regex::Regex;
use tracing::{debug, error};
use walkdir::WalkDir;

use crate::{message::Message, settings::Spy};

impl Spy {
    #[tracing::instrument]
    #[logfn(Debug)]
    pub fn new(name: String) -> Self {
        Self {
            name,
            ..Default::default()
        }
    }

    #[tracing::instrument]
    #[logfn(Debug)]
    fn notify_watch(&self, tx: mpsc::Sender<Message>) -> Result<RecommendedWatcher> {
        let spy = self.clone();
        let mut watcher = recommended_watcher(move |res| match res {
            Ok(event) => tx.send(Message::Event(event)).unwrap(),
            Err(e) => error!("watch error: {:?}", e),
        })?;
        watcher.watch(Path::new(&spy.input.unwrap()), spy.recursive)?;
        Ok(watcher)
    }

    #[tracing::instrument]
    #[logfn(Debug)]
    fn poll_watch(&self, tx: mpsc::Sender<Message>) -> Result<PollWatcher> {
        let spy = self.clone();
        let mut watcher = PollWatcher::new(
            move |res| match res {
                Ok(event) => tx.send(Message::Event(event)).unwrap(),
                Err(e) => error!("watch error: {:?}", e),
            },
            Config::default().with_poll_interval(Duration::from_millis(spy.poll.unwrap().interval)),
        )?;
        watcher.watch(Path::new(&spy.input.unwrap()), spy.recursive)?;
        Ok(watcher)
    }

    #[tracing::instrument]
    fn delay(&self, d: Option<(u64, Option<u64>)>) {
        if let Some((min, max)) = d {
            if max.is_none() {
                thread::sleep(Duration::from_millis(min));
            } else {
                let max = max.unwrap();
                let mut rng = rand::thread_rng();
                let wait = rng.gen_range(min..=max);
                thread::sleep(Duration::from_millis(wait));
            }
        }
    }

    #[tracing::instrument]
    fn watch_delay(&self) {
        self.delay(self.delay);
    }

    #[tracing::instrument]
    fn walk_delay(&self) {
        if let Some(walk) = &self.walk {
            self.delay(walk.delay);
        }
    }

    #[tracing::instrument]
    #[logfn(Debug)]
    pub fn walk(&self, tx: mpsc::Sender<Message>) -> Result<JoinHandle<()>> {
        self.walk_delay();
        let spy = self.clone();
        if spy.walk.is_none() {
            return Ok(thread::spawn(|| {}));
        }
        let walk = spy.walk.unwrap();
        let mut walker = WalkDir::new(spy.input.clone().unwrap());

        if let Some(min_path) = walk.min_depth {
            walker = walker.min_depth(min_path);
        }
        if let Some(max_path) = walk.max_depth {
            walker = walker.max_depth(max_path);
        }
        if let Some(follow_symlinks) = walk.follow_symlinks {
            walker = walker.follow_links(follow_symlinks);
        }

        let walker = walker.into_iter();

        debug!("[{}] walk input: [{}]", &spy.name, &spy.input.unwrap());
        let handle = thread::spawn(move || {
            match walk.pattern {
                Some(pattern) => {
                    debug!("[{}] walk pattern: [{}]", &spy.name, &pattern);
                    let re = Regex::new(&pattern).unwrap();
                    debug!("[{}] re: [{:?}]", &spy.name, &re);
                    walker
                        .filter_map(|e| e.ok())
                        .filter(|e| e.path().to_str().map_or(false, |s| re.is_match(s)))
                        .for_each(|e| {
                            tx.send(Message::Event(Event {
                                kind: EventKind::Modify(ModifyKind::Any),
                                paths: vec![e.path().to_path_buf()],
                                attrs: EventAttributes::new(),
                            }))
                            .unwrap();
                        });
                }
                _ => walker.filter_map(|e| e.ok()).for_each(|e| {
                    tx.send(Message::Event(Event {
                        kind: EventKind::Modify(ModifyKind::Any),
                        paths: vec![e.path().to_path_buf()],
                        attrs: EventAttributes::new(),
                    }))
                    .unwrap();
                }),
            };
        });

        Ok(handle)
    }

    #[tracing::instrument]
    pub fn watch(&self, tx: mpsc::Sender<Message>) -> Result<Box<dyn Watcher>> {
        self.watch_delay();
        match self.poll {
            Some(_) => Ok(Box::new(self.poll_watch(tx)?)),
            _ => Ok(Box::new(self.notify_watch(tx)?)),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{
        env,
        fs::{create_dir_all, remove_dir_all, File},
        sync::mpsc,
        time::Duration,
    };

    use anyhow::Result;

    use super::Spy;
    use crate::{
        message::Message,
        settings::{Poll, Walk},
    };

    #[test]
    fn test_watch() -> Result<()> {
        let tmp = env::current_dir()?.join("test");
        let watch_path = tmp.join("test_watch");
        let create_file = watch_path.join("test.txt");
        let mut spy = Spy::new("test_watch".to_string());
        spy.input = Some(watch_path.to_string_lossy().to_string());
        let (tx, rx) = mpsc::channel();
        remove_dir_all(&watch_path).unwrap_or_default();
        create_dir_all(&watch_path)?;
        let _watch = spy.watch(tx.clone())?;
        File::create(&create_file)?;

        match rx.recv_timeout(Duration::from_secs(10)) {
            Ok(message) => {
                if let Message::Event(event) = message {
                    let event_path = event.paths.last().unwrap();
                    assert_eq!(event_path.to_string_lossy(), create_file.to_string_lossy());
                } else {
                    unreachable!();
                }
            }
            Err(e) => {
                panic!("watch error: {:?}", e);
            }
        }
        Ok(())
    }

    #[test]
    fn test_poll_watch() -> Result<()> {
        let tmp = env::current_dir()?.join("test");
        let watch_path = tmp.join("test_poll_watch");
        let create_file = watch_path.join("test.txt");
        let mut spy = Spy::new("test_poll_watch".to_string());
        spy.input = Some(watch_path.to_string_lossy().to_string());
        spy.poll = Some(Poll { interval: 100 });
        let (tx, rx) = mpsc::channel();
        remove_dir_all(&watch_path).unwrap_or_default();
        create_dir_all(&watch_path)?;
        let _watch = spy.watch(tx.clone())?;
        File::create(&create_file)?;

        match rx.recv_timeout(Duration::from_secs(10)) {
            Ok(message) => {
                if let Message::Event(event) = message {
                    let event_path = event.paths.last().unwrap();
                    assert_eq!(event_path.to_string_lossy(), create_file.to_string_lossy());
                } else {
                    unreachable!();
                }
            }
            Err(e) => {
                panic!("poll watch error: {:?}", e);
            }
        }
        Ok(())
    }

    #[test]
    fn test_delay_watch() -> Result<()> {
        let tmp = env::current_dir()?.join("test");
        let watch_path = tmp.join("test_delay_watch");
        let create_file = watch_path.join("test.txt");
        let mut spy = Spy::new("test_delay_watch".to_string());
        spy.input = Some(watch_path.to_string_lossy().to_string());
        spy.delay = Some((100, Some(300)));
        let (tx, rx) = mpsc::channel();
        remove_dir_all(&watch_path).unwrap_or_default();
        create_dir_all(&watch_path)?;
        let _watch = spy.watch(tx.clone())?;
        File::create(&create_file)?;

        match rx.recv_timeout(Duration::from_secs(10)) {
            Ok(message) => {
                if let Message::Event(event) = message {
                    let event_path = event.paths.last().unwrap();
                    assert_eq!(event_path.to_string_lossy(), create_file.to_string_lossy());
                } else {
                    unreachable!();
                }
            }
            Err(e) => {
                panic!("poll watch error: {:?}", e);
            }
        }
        Ok(())
    }

    #[test]
    fn test_walk() -> Result<()> {
        let tmp = env::current_dir()?.join("test");
        let watch_path = tmp.join("test_walk");
        let create_file = watch_path.join("test.txt");
        let mut spy = Spy::new("test_walk".to_string());
        spy.input = Some(watch_path.to_string_lossy().to_string());
        spy.walk = Some(Walk {
            min_depth: Some(1),
            max_depth: Some(2),
            follow_symlinks: Some(true),
            pattern: Some("\\.*\\.txt".to_string()),
            delay: None,
        });
        let (tx, rx) = mpsc::channel();
        remove_dir_all(&watch_path).unwrap_or_default();
        create_dir_all(&watch_path)?;
        File::create(&create_file)?;
        let handle = spy.walk(tx.clone())?;

        match rx.recv_timeout(Duration::from_secs(10)) {
            Ok(message) => {
                if let Message::Event(event) = message {
                    let event_path = event.paths.last().unwrap();
                    assert_eq!(event_path.to_string_lossy(), create_file.to_string_lossy());
                } else {
                    unreachable!();
                }
            }
            Err(e) => {
                panic!("poll watch error: {:?}", e);
            }
        }

        handle.join().unwrap();
        Ok(())
    }

    #[test]
    fn test_delay_walk() -> Result<()> {
        let tmp = env::current_dir()?.join("test");
        let watch_path = tmp.join("test_delay_walk");
        let create_file = watch_path.join("test.txt");
        let mut spy = Spy::new("test_delay_walk".to_string());
        spy.input = Some(watch_path.to_string_lossy().to_string());
        spy.walk = Some(Walk {
            min_depth: Some(1),
            max_depth: Some(2),
            follow_symlinks: Some(true),
            pattern: Some("\\.*\\.txt".to_string()),
            delay: Some((100, Some(300))),
        });
        let (tx, rx) = mpsc::channel();
        remove_dir_all(&watch_path).unwrap_or_default();
        create_dir_all(&watch_path)?;
        File::create(&create_file)?;
        let handle = spy.walk(tx.clone())?;

        match rx.recv_timeout(Duration::from_secs(10)) {
            Ok(message) => {
                if let Message::Event(event) = message {
                    let event_path = event.paths.last().unwrap();
                    assert_eq!(event_path.to_string_lossy(), create_file.to_string_lossy());
                } else {
                    unreachable!();
                }
            }
            Err(e) => {
                panic!("poll watch error: {:?}", e);
            }
        }

        handle.join().unwrap();
        Ok(())
    }
}
