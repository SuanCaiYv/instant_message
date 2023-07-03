use std::{
    io::Read,
    sync::{atomic::AtomicU64, Arc},
};

use ahash::AHashMap;
use lib::{joy, Result};
use sysinfo::SystemExt;
use tracing::{error, info};

use crate::config::CONFIG;
use crate::service::get_seqnum_map;
use crate::util::{from_bytes};

mod config;
mod scheduler;
mod service;
mod util;

fn main() {
    let sys = sysinfo::System::new_all();
    tracing_subscriber::fmt()
        .event_format(
            tracing_subscriber::fmt::format()
                .with_line_number(true)
                .with_level(true)
                .with_target(true),
        )
        .with_max_level(CONFIG.log_level)
        .try_init()
        .unwrap();
    println!("{}", joy::banner());
    info!(
        "prim seqnum running on {}",
        CONFIG.server.service_address
    );
    info!("loading seqnum...");
    load().unwrap();
    info!("loading seqnum done");
    for _ in 0..sys.cpus().len() - 1 {
        std::thread::spawn(|| {
            #[cfg(target_os = "linux")]
                let _ = monoio::RuntimeBuilder::<monoio::IoUringDriver>::new()
                .with_entries(16384)
                .enable_timer()
                .build()
                .unwrap()
                .block_on(service::start());
            #[cfg(target_os = "macos")]
                let _ = monoio::RuntimeBuilder::<monoio::LegacyDriver>::new()
                .enable_timer()
                .build()
                .unwrap()
                .block_on(service::start());
        });
    }
    #[cfg(target_os = "linux")]
        let _ = monoio::RuntimeBuilder::<monoio::IoUringDriver>::new()
        .with_entries(16384)
        .enable_timer()
        .build()
        .unwrap()
        .block_on(async {
            _ = scheduler::start().await;
            service::start().await
        });
    #[cfg(target_os = "macos")]
        let _ = monoio::RuntimeBuilder::<monoio::LegacyDriver>::new()
        .enable_timer()
        .build()
        .unwrap()
        .block_on(async {
            // scheduler::start().await?;
            service::start().await
        });
}

pub(self) fn load() -> Result<()> {
    let mut map = AHashMap::new();
    let mut buf = vec![0u8; 24];
    // monoio doesn't support async read_dir, but use std is acceptable because
    // this method is only called once at the beginning of the program.
    let mut dir = std::fs::read_dir(&CONFIG.server.append_dir)?;
    while let Some(entry) = dir.next() {
        let file_name = entry?.file_name();
        if let Some(file_name_str) = file_name.to_str() {
            if file_name_str.starts_with("seqnum-") {
                let mut file = std::fs::OpenOptions::new()
                    .read(true)
                    .open(&format!("{}/{}", CONFIG.server.append_dir, file_name_str))?;
                loop {
                    let res = file.read_exact(buf.as_mut_slice());
                    if res.is_err() {
                        error!("read seqnum file error: {:?}", res);
                        break;
                    }
                    let (key, mut seq_num) = from_bytes(&buf[..]);
                    seq_num += 1;
                    map.entry(key)
                        .and_modify(|seqnum| {
                            if *seqnum < seq_num {
                                *seqnum = seq_num;
                            }
                        })
                        .or_insert(seq_num);
                }
            }
        }
    }
    let seqnum_map = get_seqnum_map();
    for (key, seqnum) in map {
        seqnum_map.insert(key, Arc::new(AtomicU64::new(seqnum)));
    }
    Ok(())
}
