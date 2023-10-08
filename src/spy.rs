// =============================================================================
// File        : spy.rs
// Author      : yukimemi
// Last Change : 2023/10/08 23:29:30.
// =============================================================================

use std::{path::Path, sync::mpsc, time::Duration};

use anyhow::Result;
use log_derive::logfn;
use notify::{recommended_watcher, Config, PollWatcher, RecommendedWatcher, Watcher};
use tracing::error;

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
    fn notify_watcher(&self, tx: mpsc::Sender<Message>) -> Result<RecommendedWatcher> {
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
    fn poll_watcher(&self, tx: mpsc::Sender<Message>) -> Result<PollWatcher> {
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
    pub fn watch(&self, tx: mpsc::Sender<Message>) -> Result<Box<dyn Watcher>> {
        match self.poll {
            Some(_) => Ok(Box::new(self.poll_watcher(tx)?)),
            _ => Ok(Box::new(self.notify_watcher(tx)?)),
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
    use crate::{message::Message, settings::Poll};

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

        match rx.recv_timeout(Duration::from_secs(1)) {
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

        match rx.recv_timeout(Duration::from_secs(1)) {
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
}