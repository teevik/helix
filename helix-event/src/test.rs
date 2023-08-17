use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use parking_lot::Mutex;

use crate::{dispatch, events, register_dynamic_hook, register_event, register_hook, Hook};
#[test]
fn smoke_test() {
    events! {
        Event1 {
            content: String
        }
        Event2 {
            content: usize
        }
    }

    #[derive(Default, Clone)]
    struct Hook1 {
        acc: Arc<Mutex<String>>,
    }
    impl Hook for Hook1 {
        type Event<'a> = Event1;
        fn run(&self, event: &mut Event1) -> Result<()> {
            self.acc.lock().push_str(&event.content);
            Ok(())
        }
    }

    #[derive(Default)]
    struct Hook2 {
        acc: Arc<AtomicUsize>,
    }
    impl Hook for Hook2 {
        type Event<'a> = Event2;
        fn run(&self, event: &mut Event2) -> Result<()> {
            self.acc.fetch_add(event.content, Ordering::Relaxed);
            Ok(())
        }
    }

    // initial registry setup
    register_event::<Event1>();
    register_event::<Event2>();

    // setup hooks
    let hook1 = Hook1::default();
    let res1 = hook1.acc.clone();
    register_hook(hook1);
    let hook2 = Hook2::default();
    let res2 = hook2.acc.clone();
    register_hook(hook2);

    // trigges events
    let thread = std::thread::spawn(|| {
        for i in 0..1000 {
            dispatch(Event2 { content: i });
        }
    });
    std::thread::sleep(Duration::from_millis(1));
    dispatch(Event1 {
        content: "foo".to_owned(),
    });
    dispatch(Event2 { content: 42 });
    dispatch(Event1 {
        content: "bar".to_owned(),
    });
    dispatch(Event1 {
        content: "hello world".to_owned(),
    });
    thread.join().unwrap();

    // check output
    assert_eq!(&**res1.lock(), "foobarhello world");
    assert_eq!(
        res2.load(Ordering::Relaxed),
        42 + (0..1000usize).sum::<usize>()
    );
}

#[test]
fn dynamic() {
    events! {
        Event3 {}
        Event4 (usize)
    };
    register_event::<Event3>();
    register_event::<Event4>();

    let count = Arc::new(AtomicUsize::new(0));
    let count1 = count.clone();
    let count2 = count.clone();
    register_dynamic_hook(
        move || {
            count1.fetch_add(2, Ordering::Relaxed);
        },
        "Event3",
    )
    .unwrap();
    register_dynamic_hook(
        move || {
            count2.fetch_add(3, Ordering::Relaxed);
        },
        "Event4",
    )
    .unwrap();
    dispatch(Event3 {});
    dispatch(Event4(0));
    dispatch(Event3 {});
    assert_eq!(count.load(Ordering::Relaxed), 7)
}
