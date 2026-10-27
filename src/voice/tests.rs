use super::*;
#[test]
fn live_beta_requires_automated_profile_but_not_manual_acceptance() {
    let mut manifest = pack::manifest();
    assert!(manifest.automated_qualified);
    manifest.release_qualified = false;
    assert!(pack::require_live_profile(&manifest).is_ok());
    manifest.automated_qualified = false;
    assert!(pack::require_live_profile(&manifest).is_err());
    manifest.release_qualified = true;
    assert!(pack::require_live_profile(&manifest).is_err());
}
use std::{
    alloc::{GlobalAlloc, Layout, System},
    cell::Cell,
};
thread_local! {static ALLOCS:Cell<Option<usize>>=const{Cell::new(None)};}
struct Counter;
#[global_allocator]
static ALLOCATOR: Counter = Counter;
unsafe impl GlobalAlloc for Counter {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let _ = ALLOCS.try_with(|c| {
            if let Some(v) = c.get() {
                c.set(Some(v + 1));
            }
        });
        System.alloc(layout)
    }
    unsafe fn dealloc(&self, p: *mut u8, layout: Layout) {
        System.dealloc(p, layout)
    }
    unsafe fn realloc(&self, p: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        let _ = ALLOCS.try_with(|c| {
            if let Some(v) = c.get() {
                c.set(Some(v + 1));
            }
        });
        System.realloc(p, layout, size)
    }
}
fn mock() -> (Rack, Node) {
    let node = Node::new(2, "Voice", NodeKind::VoiceChanger, [0.0; 2]);
    let config = Config {
        id: 2,
        voice: VoiceConfig::default(),
        active: true,
        routes: vec![],
        sources: vec![1],
    };
    let shared = Arc::new(Shared {
        input: ArrayQueue::new(64),
        output: ArrayQueue::new(64),
        messages: ArrayQueue::new(8),
        generation: AtomicU64::new(1),
        state: AtomicU8::new(READY),
        stop: AtomicBool::new(false),
        underruns: AtomicU64::new(0),
        resets: AtomicU64::new(0),
        desired: arc_swap::ArcSwapOption::empty(),
        worker_pid: AtomicU64::new(0),
        max_inference_us: AtomicU64::new(0),
        max_render_gap_us: AtomicU64::new(0),
        test_pack: std::sync::OnceLock::new(),
    });
    (
        Rack {
            shared,
            config: Some(config),
            generation: 1,
            sequence: 0,
            expected: 0,
            current: None,
            offset: 0,
            primed: true,
            fade: 0.0,
            last: [0.0; 2],
            disabled: false,
            reason: String::new(),
            last_call: None,
        },
        node,
    )
}
fn converted(shared: &Shared, generation: u64, value: f32, born: Instant) {
    for sequence in 0..30 {
        shared
            .output
            .push(Block {
                generation,
                sequence,
                born,
                frames: [[value; 2]; BLOCK_SIZE],
                valid: BLOCK_SIZE,
            })
            .ok()
            .unwrap();
    }
}
#[test]
fn adding_destinations_and_unrelated_routes_reuses_the_voice_stream() {
    let (mut rack, node) = mock();
    let mut project = Project {
        nodes: vec![
            Node::new(
                1,
                "Mic",
                NodeKind::Input {
                    device: None,
                    loopback: false,
                },
                [0.0; 2],
            ),
            node,
            Node::new(3, "Bus", NodeKind::Output { device: None }, [0.0; 2]),
        ],
        connections: vec![
            piratewave_core::project::Connection {
                input: Default::default(),
                port: Default::default(),
                from: 1,
                to: 2,
            },
            piratewave_core::project::Connection {
                input: Default::default(),
                port: Default::default(),
                from: 2,
                to: 3,
            },
        ],
        ..Default::default()
    };
    rack.sync(&project);
    let generation = rack.shared.generation.load(Ordering::Acquire);
    project.nodes.push(Node::new(
        4,
        "Second bus",
        NodeKind::Output { device: None },
        [0.0; 2],
    ));
    project
        .connections
        .push(piratewave_core::project::Connection {
            input: Default::default(),
            port: Default::default(),
            from: 2,
            to: 4,
        });
    project
        .nodes
        .push(Node::new(5, "Pads", NodeKind::Soundboard, [0.0; 2]));
    project
        .connections
        .push(piratewave_core::project::Connection {
            input: Default::default(),
            port: Default::default(),
            from: 5,
            to: 4,
        });
    project.nodes.reverse();
    project.connections.reverse();
    rack.sync(&project);
    assert_eq!(rack.shared.generation.load(Ordering::Acquire), generation);
    project.nodes.iter_mut().find(|n| n.id == 1).unwrap().kind = NodeKind::Input {
        device: Some("different microphone".into()),
        loopback: false,
    };
    rack.sync(&project);
    assert!(rack.shared.generation.load(Ordering::Acquire) > generation);
}
#[test]
fn bypassed_voice_nodes_do_not_share_the_active_nodes_fade_or_audio() {
    let (mut rack, node) = mock();
    converted(&rack.shared, 1, 0.1, Instant::now());
    let mut frames = [[0.8; 2]; BLOCK_SIZE];
    rack.process(&node, &mut frames);
    let fade = rack.fade;
    let offset = rack.offset;
    let last = rack.last;
    let mut bypassed = Node::new(99, "Dry", NodeKind::VoiceChanger, [0.0; 2]);
    bypassed.effects_disabled = true;
    frames.fill([0.8; 2]);
    ALLOCS.with(|v| v.set(Some(0)));
    rack.process(&bypassed, &mut frames);
    let allocations = ALLOCS.with(|v| v.replace(None).unwrap());
    assert_eq!(allocations, 0);
    assert!(frames.iter().flatten().all(|v| *v == 0.8));
    assert_eq!((rack.fade, rack.offset, rack.last), (fade, offset, last));
    assert!(!rack.disabled);
    rack.process(&node, &mut frames);
    assert!(frames.iter().flatten().all(|v| *v <= 0.1));
}
#[test]
fn adapter_is_allocation_free_and_rejects_stale_generations() {
    let (mut rack, node) = mock();
    converted(&rack.shared, 1, 0.1, Instant::now());
    let mut frames = [[0.8; 2]; BLOCK_SIZE];
    ALLOCS.with(|v| v.set(Some(0)));
    rack.process(&node, &mut frames);
    let count = ALLOCS.with(|v| v.replace(None).unwrap());
    assert_eq!(count, 0);
    assert!(frames[127][0] > 0.05 && frames[127][0] < 0.11);
    rack.restart();
    frames.fill([0.8; 2]);
    rack.process(&node, &mut frames);
    assert!(frames.iter().flatten().all(|v| *v == 0.0));
    converted(&rack.shared, 2, 0.2, Instant::now());
    rack.shared.state.store(PRIMING, Ordering::Release);
    rack.process(&node, &mut frames);
    assert!(frames[127][0] > 0.1 && frames[127][0] < 0.21);
}
#[test]
fn stalls_and_old_audio_recover_without_passing_dry_input() {
    let (mut rack, node) = mock();
    for sequence in 0..64 {
        rack.shared
            .input
            .push(Block {
                generation: 1,
                sequence,
                born: Instant::now(),
                frames: [[0.5; 2]; BLOCK_SIZE],
                valid: BLOCK_SIZE,
            })
            .ok()
            .unwrap();
    }
    let mut frames = [[0.8; 2]; BLOCK_SIZE];
    rack.process(&node, &mut frames);
    assert_eq!(rack.shared.generation.load(Ordering::Acquire), 2);
    assert!(frames.iter().flatten().all(|v| *v == 0.0));
    let (mut rack, node) = mock();
    converted(
        &rack.shared,
        1,
        0.1,
        Instant::now() - Duration::from_millis(300),
    );
    rack.process(&node, &mut frames);
    assert_eq!(rack.shared.generation.load(Ordering::Acquire), 2);
    assert!(frames.iter().flatten().all(|v| *v == 0.0));
}
#[test]
fn only_upstream_capture_discontinuities_reset_the_voice() {
    let (rack, _) = mock();
    rack.source_discontinuity(99);
    assert_eq!(rack.shared.generation.load(Ordering::Acquire), 1);
    ALLOCS.with(|v| v.set(Some(0)));
    rack.source_discontinuity(1);
    let allocations = ALLOCS.with(|v| v.replace(None).unwrap());
    assert_eq!(allocations, 0);
    assert_eq!(rack.shared.generation.load(Ordering::Acquire), 2);
}
#[test]
fn unavailable_gpu_fails_instead_of_falling_back() {
    let mut manifest = pack::manifest();
    manifest.directml_qualified = false;
    assert!(pack::require_backend(&manifest, VoiceBackend::DirectMl).is_err());
    assert!(pack::require_backend(&manifest, VoiceBackend::Cpu).is_ok());
    assert!(pack::require_backend(&manifest, VoiceBackend::Auto).is_ok());
    manifest.directml_qualified = true;
    assert!(pack::require_backend(&manifest, VoiceBackend::DirectMl).is_ok());
    manifest.automated_qualified = false;
    assert!(pack::require_backend(&manifest, VoiceBackend::DirectMl).is_err());
}
#[test]
fn stalled_voice_does_not_interrupt_soundboard_application_or_other_buses() {
    use piratewave_core::{effects::EffectRegistry, graph::Graph, project::Connection};
    let (mut rack, node) = mock();
    let p = Project {
        nodes: vec![
            Node::new(
                1,
                "Mic",
                NodeKind::Input {
                    device: None,
                    loopback: false,
                },
                [0.0; 2],
            ),
            node,
            Node::new(3, "Soundboard", NodeKind::Soundboard, [0.0; 2]),
            Node::new(4, "Voice bus", NodeKind::Output { device: None }, [0.0; 2]),
            Node::new(5, "Music bus", NodeKind::Output { device: None }, [0.0; 2]),
            Node::new(
                6,
                "Application",
                NodeKind::ApplicationCapture { target: None },
                [0.0; 2],
            ),
        ],
        connections: vec![
            Connection {
                input: Default::default(),
                port: Default::default(),
                from: 1,
                to: 2,
            },
            Connection {
                input: Default::default(),
                port: Default::default(),
                from: 2,
                to: 4,
            },
            Connection {
                input: Default::default(),
                port: Default::default(),
                from: 3,
                to: 5,
            },
            Connection {
                input: Default::default(),
                port: Default::default(),
                from: 6,
                to: 5,
            },
        ],
        ..Default::default()
    };
    let mut graph = Graph::new(&p, &EffectRegistry::default()).unwrap();
    let blocks = [
        (1, [[0.8; 2]; BLOCK_SIZE]),
        (3, [[0.2; 2]; BLOCK_SIZE]),
        (6, [[0.1; 2]; BLOCK_SIZE]),
    ];
    for _ in 0..100 {
        graph.process_with_external(&blocks, |node, frames| {
            rack.process(node, frames);
            true
        });
        assert!(graph
            .nodes
            .iter()
            .find(|n| n.config.id == 4)
            .unwrap()
            .frames
            .iter()
            .flatten()
            .all(|v| *v == 0.0));
        let bus = &graph
            .nodes
            .iter()
            .find(|n| n.config.id == 5)
            .unwrap()
            .frames;
        assert!(bus.iter().flatten().all(|v| (*v - 0.3).abs() < 1e-6));
    }
}
#[test]
#[ignore = "Requires the locally built optional native voice pack; no devices or user projects are changed"]
fn native_voice_pipeline() {
    let _audio_thread = crate::audio_scheduling::AudioThread::register();
    let path = std::path::PathBuf::from(
        std::env::var("PIRATEWAVE_VOICE_TEST_PACK").expect("Set PIRATEWAVE_VOICE_TEST_PACK"),
    );
    let duration = std::env::var("PIRATEWAVE_VOICE_TEST_SECONDS")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(20);
    let mut p = Project::default();
    p.nodes.clear();
    p.connections.clear();
    let mut node = Node::new(2, "Voice", NodeKind::VoiceChanger, [0.0; 2]);
    if std::env::var("PIRATEWAVE_VOICE_TEST_BACKEND").as_deref() == Ok("directml") {
        node.voice.get_or_insert_default().backend = VoiceBackend::DirectMl;
    }
    p.nodes.push(node.clone());
    p.nodes.push(Node::new(
        3,
        "Bus",
        NodeKind::Output { device: None },
        [0.0; 2],
    ));
    p.connections.push(piratewave_core::project::Connection {
        input: Default::default(),
        port: Default::default(),
        from: 2,
        to: 3,
    });
    let mut empty = p.clone();
    empty.nodes.clear();
    empty.connections.clear();
    let mut rack = Rack::new(&empty);
    rack.shared.test_pack.set(path.clone()).unwrap();
    rack.sync(&p);
    let until = Instant::now() + Duration::from_secs(15);
    while Instant::now() < until && rack.shared.state.load(Ordering::Acquire) != PRIMING {
        thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(
        rack.shared.state.load(Ordering::Acquire),
        PRIMING,
        "{:?}",
        rack.status()
    );
    let start = Instant::now();
    let mut deadline = start;
    let mut blocks = 0;
    let mut first_ready = None;
    let mut nonzero = false;
    let initial_generation = rack.shared.generation.load(Ordering::Acquire);
    let mut next_memory = Duration::from_secs(60);
    let mut memory = vec![];
    let trace = std::env::var("PIRATEWAVE_VOICE_TEST_TRACE").ok();
    if trace.is_some() {
        assert!(duration <= 120, "Trace capture is limited to two minutes");
    }
    let capacity = if trace.is_some() {
        duration as usize * 48000 + BLOCK_SIZE
    } else {
        0
    };
    let mut input_trace = Vec::with_capacity(capacity);
    let mut output_trace = Vec::with_capacity(capacity);
    let mut first_ready_sample = None;
    while start.elapsed() < Duration::from_secs(duration) {
        let mut frames = std::array::from_fn::<_, BLOCK_SIZE, _>(|i| {
            let t = (blocks * BLOCK_SIZE + i) as f32 / 48000.0;
            let s = (std::f32::consts::TAU * 160.0 * t).sin() * 0.12;
            [s, s]
        });
        if trace.is_some() {
            input_trace.extend(frames.iter().map(|f| f[0]));
        }
        rack.process(&node, &mut frames);
        if trace.is_some() {
            output_trace.extend(frames.iter().map(|f| f[0]));
        }
        if rack.shared.state.load(Ordering::Acquire) == READY {
            if first_ready.is_none() {
                first_ready_sample = Some(blocks * BLOCK_SIZE);
                let elapsed = start.elapsed();
                assert!(
                    elapsed < Duration::from_millis(100),
                    "First ready {elapsed:?}"
                );
                println!("First ready: {elapsed:?}");
            }
            first_ready.get_or_insert(start.elapsed());
        }
        if first_ready.is_some() {
            assert_eq!(
                rack.shared.state.load(Ordering::Acquire),
                READY,
                "Voice lost readiness: {:?}",
                rack.status()
            );
            assert_eq!(
                rack.shared.generation.load(Ordering::Acquire),
                initial_generation,
                "Unexpected voice reset"
            );
        }
        nonzero |= frames.iter().flatten().any(|v| v.abs() > 1e-5);
        if start.elapsed() >= next_memory {
            if let Some(bytes) =
                private_bytes(rack.shared.worker_pid.load(Ordering::Acquire) as u32)
            {
                memory.push(bytes);
            }
            println!(
                "Native pipeline: {}s, underruns {}, private bytes {:?}",
                start.elapsed().as_secs(),
                rack.shared.underruns.load(Ordering::Relaxed),
                memory.last()
            );
            next_memory += Duration::from_secs(60);
        }
        blocks += 1;
        deadline += Duration::from_secs_f64(BLOCK_SIZE as f64 / 48000.0);
        if let Some(wait) = deadline.checked_duration_since(Instant::now()) {
            thread::sleep(wait);
        }
    }
    let ready = first_ready.expect("No converted audio became ready");
    assert!(ready < Duration::from_millis(100), "First ready {ready:?}");
    assert!(nonzero);
    if duration >= 1800 {
        assert!(memory.len() >= 20);
        assert!(
            memory.iter().max().unwrap() - memory.iter().min().unwrap() < 64 * 1024 * 1024,
            "Worker memory growth: {memory:?}"
        );
    }
    assert_eq!(
        rack.shared.underruns.load(Ordering::Relaxed),
        0,
        "{:?}",
        rack.status()
    );
    println!(
        "Native pipeline: {duration}s, first ready {ready:?}, blocks {blocks}, status {:?}",
        rack.status()
    );
    if let Ok(path) = std::env::var("PIRATEWAVE_VOICE_TEST_REPORT") {
        let worker = if node
            .voice
            .as_ref()
            .is_some_and(|v| v.backend == VoiceBackend::DirectMl)
        {
            "PatchVoiceDmlWorker.exe"
        } else {
            "PatchVoiceWorker.exe"
        };
        std::fs::write(path,serde_json::to_string_pretty(&serde_json::json!({"model_sha256":pack::manifest().model_sha256,"worker_sha256":pack::manifest().files[worker].sha256,"backend":node.voice.as_ref().map(|v|v.backend),"duration_seconds":duration,"first_ready_ms":ready.as_secs_f64()*1000.0,"blocks":blocks,"underruns":rack.shared.underruns.load(Ordering::Relaxed),"worker_private_bytes":memory,"max_inference_ms":rack.shared.max_inference_us.load(Ordering::Relaxed) as f64 / 1000.0,"max_render_gap_ms":rack.shared.max_render_gap_us.load(Ordering::Relaxed) as f64 / 1000.0,"pass":true})).unwrap()).unwrap();
    }
    if let Some(prefix) = trace {
        use std::io::Write;
        for (suffix, values) in [("input", input_trace), ("output", output_trace)] {
            let mut file = std::fs::File::create(format!("{prefix}.{suffix}.f32")).unwrap();
            for value in values {
                file.write_all(&value.to_le_bytes()).unwrap();
            }
        }
        std::fs::write(
            format!("{prefix}.json"),
            serde_json::to_string_pretty(
                &serde_json::json!({"first_ready_sample":first_ready_sample,"sample_rate":48000}),
            )
            .unwrap(),
        )
        .unwrap();
    }
    rack.restart();
    let mut frames = [[0.8; 2]; BLOCK_SIZE];
    rack.process(&node, &mut frames);
    assert!(frames.iter().flatten().all(|v| *v == 0.0));
}
#[cfg(windows)]
fn private_bytes(pid: u32) -> Option<usize> {
    use windows_sys::Win32::{
        Foundation::{CloseHandle, HANDLE},
        System::Threading::{OpenProcess, PROCESS_QUERY_INFORMATION, PROCESS_VM_READ},
    };
    #[repr(C)]
    struct Counters {
        cb: u32,
        faults: u32,
        values: [usize; 9],
    }
    #[link(name = "kernel32")]
    extern "system" {
        #[link_name = "K32GetProcessMemoryInfo"]
        fn process_memory_info(handle: HANDLE, data: *mut Counters, bytes: u32) -> i32;
    }
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_INFORMATION | PROCESS_VM_READ, 0, pid);
        if handle.is_null() {
            return None;
        }
        let mut data = Counters {
            cb: std::mem::size_of::<Counters>() as u32,
            faults: 0,
            values: [0; 9],
        };
        let ok = process_memory_info(handle, &mut data, data.cb) != 0;
        CloseHandle(handle);
        ok.then_some(data.values[8])
    }
}
#[cfg(not(windows))]
fn private_bytes(_: u32) -> Option<usize> {
    None
}
#[cfg(windows)]
#[test]
#[ignore = "Requires optional pack; kills only its own native worker; no audio devices"]
fn native_worker_lifecycle() {
    let path = std::path::PathBuf::from(std::env::var("PIRATEWAVE_VOICE_TEST_PACK").unwrap());
    let mut project = Project::default();
    project.nodes = vec![
        Node::new(2, "Voice", NodeKind::VoiceChanger, [0.0; 2]),
        Node::new(3, "Bus", NodeKind::Output { device: None }, [0.0; 2]),
    ];
    project.connections = vec![piratewave_core::project::Connection {
        input: Default::default(),
        port: Default::default(),
        from: 2,
        to: 3,
    }];
    if std::env::var("PIRATEWAVE_VOICE_TEST_BACKEND").as_deref() == Ok("directml") {
        project.nodes[0].voice.get_or_insert_default().backend = VoiceBackend::DirectMl;
    }
    let mut rack = Rack::new(&Project {
        nodes: vec![],
        connections: vec![],
        ..Default::default()
    });
    rack.shared.test_pack.set(path).unwrap();
    rack.sync(&project);
    let wait_for = |rack: &Rack, state| {
        // Debug SHA verification of the full 280 MB pack is much slower than
        // optimized release; inference still has its separate 15s deadline.
        let until = Instant::now() + Duration::from_secs(60);
        while Instant::now() < until
            && (rack.shared.state.load(Ordering::Acquire) != state
                || (state == DISABLED && rack.shared.worker_pid.load(Ordering::Acquire) != 0))
        {
            thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(rack.shared.state.load(Ordering::Acquire), state);
    };
    wait_for(&rack, PRIMING);
    let worker_pid = rack.shared.worker_pid.load(Ordering::Acquire) as u32;
    assert_ne!(worker_pid, 0);
    for _ in 0..50 {
        let mut frames = [[0.2; 2]; BLOCK_SIZE];
        rack.process(&project.nodes[0], &mut frames);
        thread::sleep(Duration::from_micros(2667));
    }
    assert_eq!(rack.shared.state.load(Ordering::Acquire), READY);
    // This PID came directly from this rack's private child handle.
    unsafe {
        use windows_sys::Win32::{
            Foundation::CloseHandle,
            System::Threading::{OpenProcess, TerminateProcess, PROCESS_TERMINATE},
        };
        let process = OpenProcess(PROCESS_TERMINATE, 0, worker_pid);
        assert!(!process.is_null());
        assert_ne!(TerminateProcess(process, 17), 0);
        CloseHandle(process);
    }
    wait_for(&rack, ERROR);
    let mut frames = [[0.8; 2]; BLOCK_SIZE];
    rack.process(&project.nodes[0], &mut frames);
    assert!(frames.iter().flatten().all(|v| *v == 0.0));
    // No manual restart: the supervisor relaunches the same configured backend.
    wait_for(&rack, PRIMING);
    assert_ne!(
        rack.shared.worker_pid.load(Ordering::Acquire),
        u64::from(worker_pid)
    );
    project.nodes[0].effects_disabled = true;
    rack.sync(&project);
    wait_for(&rack, DISABLED);
    assert_eq!(rack.shared.worker_pid.load(Ordering::Acquire), 0);
    for _ in 0..3 {
        frames.fill([0.8; 2]);
        rack.process(&project.nodes[0], &mut frames);
    }
    assert!(frames.iter().flatten().all(|v| (*v - 0.8).abs() < 1e-6));
    project.nodes[0].effects_disabled = false;
    rack.sync(&project);
    wait_for(&rack, PRIMING);
    project.connections.clear();
    rack.sync(&project);
    wait_for(&rack, DISABLED);
    assert_eq!(rack.shared.worker_pid.load(Ordering::Acquire), 0);
}

#[test]
fn recording_is_an_active_voice_destination_even_without_live_output_bus() {
    let (mut rack, voice) = mock();
    let mut recording = Node::new(98, "Recorder", NodeKind::Recording, [0.; 2]);
    recording.recording.enabled = true;
    recording
        .recording
        .slots
        .push(piratewave_core::recording::RecordSlot::new("Slot"));
    let mut project = Project {
        nodes: vec![voice, recording],
        connections: vec![],
        ..Project::default()
    };
    project.connect(project.nodes[0].id, 98).unwrap();
    rack.sync(&project);
    assert!(rack.config.as_ref().unwrap().active);
    project.nodes[1].recording.enabled = false;
    rack.sync(&project);
    assert!(!rack.config.as_ref().unwrap().active);
}

#[test]
fn switch_selects_one_voice_runtime_and_invalidates_old_results() {
    use piratewave_core::project::{Connection, Project};
    let (mut rack, _) = mock();
    let mut project = Project {
        nodes: vec![
            Node::new(1, "Source", NodeKind::Soundboard, [0.; 2]),
            Node::new(2, "Choose", NodeKind::Switch, [0.; 2]),
            Node::new(3, "Voice A", NodeKind::VoiceChanger, [0.; 2]),
            Node::new(4, "Voice B", NodeKind::VoiceChanger, [0.; 2]),
            Node::new(
                5,
                "Bus",
                NodeKind::Output {
                    device: Some(String::new()),
                },
                [0.; 2],
            ),
        ],
        connections: [(1, 2), (2, 3), (2, 4), (3, 5), (4, 5)]
            .into_iter()
            .map(|(from, to)| Connection {
                from,
                to,
                port: Default::default(),
                input: Default::default(),
            })
            .collect(),
        ..Project::default()
    };
    piratewave_core::graph::validate(&project).unwrap();
    rack.sync(&project);
    assert_eq!(rack.config.as_ref().unwrap().id, 3);
    assert!(rack.config.as_ref().unwrap().active);
    let old_generation = rack.shared.generation.load(Ordering::Acquire);
    converted(&rack.shared, old_generation, 0.7, Instant::now());
    assert!(project.select_switch_output(2, 4));
    piratewave_core::graph::validate(&project).unwrap();
    rack.sync(&project);
    assert_eq!(rack.config.as_ref().unwrap().id, 4);
    assert!(rack.shared.generation.load(Ordering::Acquire) > old_generation);
    let mut frames = [[0.5; 2]; 128];
    rack.process(&project.nodes[2], &mut frames);
    assert!(frames.iter().flatten().all(|s| *s == 0.));
    rack.process(&project.nodes[3], &mut frames);
    assert!(
        frames.iter().flatten().all(|s| *s == 0.),
        "No previous voice audio after switching"
    );
    // An extra non-exclusive input cannot evade the single-runtime constraint.
    project.connections.push(Connection {
        from: 1,
        to: 3,
        port: Default::default(),
        input: Default::default(),
    });
    assert!(piratewave_core::graph::validate(&project).is_err());
    project.connections.pop();
    project.nodes[1].switch_output = Some(3);
    rack.sync(&project);
    assert_eq!(rack.config.as_ref().unwrap().id, 3);
}
