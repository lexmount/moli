use std::{collections::BTreeMap, fs, path::Path};

use anyhow::{Context, Result, bail, ensure};

use super::{MAX_SEQUENCE, coding::Decoder, log::read_records};

#[derive(Default)]
pub(super) struct Manifest {
    pub tables: BTreeMap<(u32, u64), u64>,
    pub log_number: u64,
    pub previous_log_number: u64,
    pub last_sequence: u64,
}

impl Manifest {
    pub fn read(directory: &Path) -> Result<Self> {
        let current = fs::read_to_string(directory.join("CURRENT"))
            .context("failed to read LevelDB CURRENT")?;
        let name = current
            .strip_suffix('\n')
            .context("LevelDB CURRENT must end with a newline")?;
        let number = name
            .strip_prefix("MANIFEST-")
            .context("invalid LevelDB CURRENT manifest name")?;
        ensure!(
            !number.is_empty() && number.bytes().all(|byte| byte.is_ascii_digit()),
            "invalid LevelDB CURRENT manifest name"
        );
        let mut manifest = Self::default();
        let mut has_log = false;
        let mut has_next_file = false;
        let mut has_last_sequence = false;
        read_records(&directory.join(name), |record| {
            let mut input = Decoder::new(record);
            let mut deleted = Vec::new();
            let mut added = Vec::new();
            while !input.bytes.is_empty() {
                match input.varint32()? {
                    1 => ensure!(
                        input.slice()? == b"leveldb.BytewiseComparator",
                        "unsupported LevelDB comparator"
                    ),
                    2 => {
                        manifest.log_number = input.varint()?;
                        has_log = true;
                    }
                    3 => {
                        input.varint()?;
                        has_next_file = true;
                    }
                    4 => {
                        manifest.last_sequence = input.varint()?;
                        ensure!(
                            manifest.last_sequence <= MAX_SEQUENCE,
                            "invalid LevelDB sequence number"
                        );
                        has_last_sequence = true;
                    }
                    5 => {
                        read_level(&mut input)?;
                        ensure!(input.slice()?.len() >= 8, "invalid LevelDB compaction key");
                    }
                    6 => deleted.push((read_level(&mut input)?, input.varint()?)),
                    7 => {
                        let level = read_level(&mut input)?;
                        let number = input.varint()?;
                        let size = input.varint()?;
                        ensure!(input.slice()?.len() >= 8, "invalid LevelDB smallest key");
                        ensure!(input.slice()?.len() >= 8, "invalid LevelDB largest key");
                        added.push(((level, number), size));
                    }
                    9 => manifest.previous_log_number = input.varint()?,
                    tag => bail!("unsupported LevelDB manifest tag {tag}"),
                }
            }
            // A VersionEdit applies deletions before additions, including a
            // table moved between levels without rewriting its contents.
            for key in deleted {
                manifest.tables.remove(&key);
            }
            manifest.tables.extend(added);
            Ok(())
        })
        .with_context(|| format!("failed to read LevelDB manifest `{name}`"))?;
        ensure!(
            has_log && has_next_file && has_last_sequence,
            "incomplete LevelDB manifest"
        );
        Ok(manifest)
    }
}

fn read_level(input: &mut Decoder<'_>) -> Result<u32> {
    let level = input.varint32()?;
    ensure!(level < 7, "unsupported LevelDB level {level}");
    Ok(level)
}
