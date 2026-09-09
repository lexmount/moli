/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

use std::{env, fs, path::Path};

use toml::{Table, Value};

struct BooleanPreference {
    name: String,
    default: bool,
}

struct IntegerPreference {
    name: String,
    default: i64,
}

fn main() -> Result<(), std::io::Error> {
    println!("cargo::rerun-if-changed=preferences.toml");
    generate_code(parse_preferences()?)
}

fn parse_preferences() -> Result<(Vec<BooleanPreference>, Vec<IntegerPreference>), std::io::Error> {
    let preferences_text = fs::read_to_string("preferences.toml")?;
    let toml = preferences_text
        .parse::<Table>()
        .expect("Could not parse preferences.toml");

    let mut boolean_preferences = Vec::new();
    let mut integer_preferences = Vec::new();
    for (name, default) in toml {
        match default {
            Value::Boolean(default) => {
                boolean_preferences.push(BooleanPreference { name, default });
            },
            Value::Integer(default) => {
                integer_preferences.push(IntegerPreference { name, default });
            },
            _ => panic!("Found unknown preference type: {default:?}"),
        }
    }

    Ok((boolean_preferences, integer_preferences))
}

fn generate_code(
    preferences: (Vec<BooleanPreference>, Vec<IntegerPreference>),
) -> Result<(), std::io::Error> {
    let mut output = Vec::new();

    let (boolean_preferences, integer_preferences) = preferences;
    let boolean_count = boolean_preferences.len();
    output.push(format!(
        "pub static BOOLS: [AtomicBool; {boolean_count}] = ["
    ));
    for preference in &boolean_preferences {
        output.push(format!("    AtomicBool::new({}),", preference.default));
    }
    output.push(format!("];"));

    let integer_count = integer_preferences.len();
    output.push(format!(
        "pub static INTEGERS: [AtomicI32; {integer_count}] = ["
    ));
    for preference in &integer_preferences {
        output.push(format!("    AtomicI32::new({}),", preference.default));
    }
    output.push(format!("];"));

    output.push(format!(
        "/// Returns the value of a preference exposed to the style crate. If the embedder"
    ));
    output.push(format!(
        "/// has not set a value for it, this returns the default value of the preference"
    ));
    output.push(format!("#[macro_export]"));
    output.push(format!("macro_rules! pref {{"));

    for (index, preference) in boolean_preferences.iter().enumerate() {
        output.push(format!("    ({:?}) => {{", preference.name));
        output.push(format!(
            "        $crate::BOOLS[{index:?}].load(std::sync::atomic::Ordering::Relaxed)",
        ));
        output.push(format!("    }};"));
    }

    for (index, preference) in integer_preferences.iter().enumerate() {
        output.push(format!("    ({:?}) => {{", preference.name));
        output.push(format!(
            "        $crate::INTEGERS[{index:?}].load(std::sync::atomic::Ordering::Relaxed)",
        ));
        output.push(format!("    }};"));
    }
    output.push(format!("}}"));

    output.push(format!("#[macro_export]"));
    output.push(format!("macro_rules! set_pref {{"));

    for (index, preference) in boolean_preferences.iter().enumerate() {
        output.push(format!(
            "    ({:?}, $($value:expr)+) => {{",
            preference.name
        ));

        output.push(format!("let value = $($value)+;"));
        output.push(format!(
            "        $crate::BOOLS[{index:?}].store(value, std::sync::atomic::Ordering::Relaxed)",
        ));
        output.push(format!("    }};"));
    }

    for (index, preference) in integer_preferences.iter().enumerate() {
        output.push(format!(
            "    ({:?}, $($value:expr)+) => {{",
            preference.name
        ));
        output.push(format!("let value = $($value)+;"));
        output.push(format!(
            "        $crate::INTEGERS[{index:?}].store(value, std::sync::atomic::Ordering::Relaxed)",
        ));
        output.push(format!("    }};"));
    }
    output.push(format!("}}"));

    let output = output.join("\n");
    println!("{output}");

    let out_dir = env::var_os("OUT_DIR").expect("Should always have OUT_DIR set");
    fs::write(Path::new(&out_dir).join("generated.rs"), output)
}
