//! Times reading an app's main-window accessibility tree, batched and one attribute per call.
//! Prints counts and milliseconds only, never the tree's content. Read-only.
//!
//! `cargo run -p ottospace-mac --example ax_bench -- <pid> [nodes] [depth]`

use std::time::Instant;

use ottospace_core::ax::{self, Limits};
use ottospace_mac::ax::Window;
use ottospace_mac::ax_tree::{Fetch, read};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(pid) = args.first().and_then(|a| a.parse().ok()) else {
        eprintln!("usage: ax_bench <pid> [nodes] [depth]");
        std::process::exit(2);
    };
    let defaults = Limits::default();
    let nodes = args.get(1).and_then(|a| a.parse().ok()).unwrap_or(defaults.nodes);
    let depth = args.get(2).and_then(|a| a.parse().ok()).unwrap_or(defaults.depth);
    let Some(window) = Window::main_of(pid) else {
        eprintln!("pid {pid} has no standard window");
        std::process::exit(1);
    };
    // Batched twice: the first read of an app can pay for it building its tree.
    for fetch in [Fetch::Batched, Fetch::Single, Fetch::Batched] {
        let start = Instant::now();
        let tree = read(&window, Limits { depth, nodes, millis: None }, fetch);
        let ms = start.elapsed().as_secs_f64() * 1000.0;
        let interactive = ax::flatten(&tree.root).iter().filter(|f| ax::is_interactive(f.node)).count();
        println!(
            "{fetch:?}: {} nodes, {interactive} interactive, {ms:.0} ms ({:.2} ms/node), truncated {:?}",
            tree.count,
            ms / tree.count.max(1) as f64,
            tree.truncated
        );
    }
}
