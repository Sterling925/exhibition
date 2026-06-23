// SPDX-License-Identifier: MIT OR Apache-2.0
// SPDX-FileCopyrightText: 2026 https://github.com/Sterling925

//! Exhibition CLI entry point.
//!
//! Reads paths interactively from stdin, parses the user's `.ioc` file via
//! [`ioc_parser`], picks an RT transport via [`engine::select_transport`],
//! and writes the generated `exhibition_rt/` tree under the project path
//! supplied by the user. If the user opts in, also writes a companion
//! `exhibition_bc/` host-side project under a separate output folder.

mod device_definitions;
mod engine;
mod gen_log;
mod helper_functions;
mod ioc_parser;

#[cfg(test)]
mod tests;

use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use gen_log::GenLog;

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("Error: {}", e);
            ExitCode::from(1)
        }
    }
}


fn run() -> Result<(), String> {
    println!("Exhibition: STM32CubeMX -> RT firmware + BC host generator");

    // Accumulates the generation report written into the RT project so the
    // generated code carries a record of what was enabled and why.
    let mut log = GenLog::new();

    let project_path = prompt_for_path("Path to the generated STM32CubeMX project root: ")?;

    let ioc_path = match find_ioc_in_project(&project_path) {
        Some(found) => {
            println!("Found .ioc file: {}", found.display());
            found
        }
        None => prompt_for_path("Path to the .ioc file: ")?,
    };

    log.source(format!("`.ioc` file: `{}`", ioc_path.display()));

    let devices = ioc_parser::parse_path_logged(&ioc_path, &mut log)
        .map_err(|e| format!("Failed to parse '{}': {}", ioc_path.display(), e))?;

    log.source(format!(
        "Target MCU: {}",
        devices.core.as_ref().map(|c| c.model()).unwrap_or("(unknown)"),
    ));

    let transport = engine::select_transport(&devices)?;
    let detail = transport.hal_handle()
        .map(|h| format!("HAL handle {}", h))
        .or_else(|| transport.transmit_fn_name().map(|f| format!("transmit fn {}", f)))
        .unwrap_or_else(|| "no auxiliary handle".to_string());
    println!(
        "Selected transport: {} on '{}' ({})",
        transport.kind(),
        transport.bus_name(),
        detail,
    );
    log.source(format!(
        "Transport: {} on `{}` ({})",
        transport.kind(),
        transport.bus_name(),
        detail,
    ));

    // Build the shared artifacts once so RT and BC are generated from the
    // same sub-address assignment, manifest, and fingerprint.
    let art = engine::build_artifacts_logged(&devices, &transport, &mut log)?;

    let patch = engine::generate_rt_logged(&art, &transport, &project_path, &mut log)?;

    let src_rel = engine::codegen::source_dir_rel(&devices.layout);
    let inc_subdir = project_path.join(engine::codegen::INC_DIR_REL);
    let src_subdir = project_path.join(&src_rel);
    println!("\nGenerated RT files under:");
    println!("  headers:        {}", inc_subdir.display());
    println!("  implementations: {}", src_subdir.display());
    println!("  generation log:  {}", src_subdir.join("api/generation_report.md").display());
    if !engine::codegen::layout_is_modeled(&devices.layout) {
        println!(
            "  NOTE: this is an UnderRoot=false project with toolchain '{}', whose \
             on-disk layout exhibition does not model. Sources were written to the \
             default '{}'; move them to your toolchain's source folder if it builds \
             from elsewhere.",
            devices.layout.toolchain, src_rel,
        );
    }
    println!("RT next steps:");
    println!("  1. In STM32CubeIDE, refresh the project (right-click \u{2192} Refresh, or F5)");
    println!("     so the indexer picks up the new files under {} and {}.",
             engine::codegen::INC_DIR_REL, src_rel);
    print_main_wiring_step(&patch);
    println!("  3. Drive the RT from a BC connected to the selected transport.");

    if prompt_yes_no("\nAlso generate the BC (host) project? [Y/n]: ")? {
        let bc_path = prompt_for_path("Path to the BC project output root: ")?;
        engine::generate_bc(&art, &transport, &bc_path)?;
        let bc_subdir = bc_path.join(engine::bc_codegen::BC_OUTPUT_DIR_NAME);
        println!("\nGenerated BC project at: {}", bc_subdir.display());
        println!("BC next steps:");
        println!("  1. cd {}", bc_subdir.display());
        println!("  2. cmake --preset Debug && cmake --build --preset Debug");
        println!("  3. ./build/Debug/exhibition_bc <serial-device> [<baud>]");

        // Use the parsed `.ioc` baud rate in the example commands when it
        // is known; otherwise fall back to 115200 and (for a UART, not for
        // USB CDC) print a note flagging the value as a guess.
        let baud = transport.baud_rate().unwrap_or(115200);
        println!("       Linux UART:    ./build/Debug/exhibition_bc /dev/ttyUSB0 {}", baud);
        println!("       Linux USB CDC: ./build/Debug/exhibition_bc /dev/ttyACM0 {}", baud);
        println!("       Windows:       .\\build\\Debug\\exhibition_bc.exe COM7 {}", baud);
        if matches!(
            transport,
            engine::TransportChoice::Uart { baud_rate: None, .. },
        ) {
            println!(
                "       NOTE: .ioc did not specify a baud rate. {} is a fallback — \
                 pass the real baud as the third argument if it differs.",
                baud,
            );
        }
    }

    Ok(())
}


/// Print step 2 of the "RT next steps" list, describing how the three RT
/// integration lines reached `main.c`.
///
/// `patch` is what [`engine::generate_rt_logged`] reported:
/// - `Some` & every line wired (inserted or already present): the generator did
///   it; just rebuild — no manual edit, so the step is a confirmation.
/// - `Some` but a USER CODE region was missing: name the line(s) the user still
///   has to paste in by hand and where.
/// - `None` (no `main.c`/`main.cpp` found): fall back to the full manual
///   instruction.
fn print_main_wiring_step(
    patch: &Option<(PathBuf, engine::main_patch::PatchReport)>,
) {
    use engine::main_patch::LineStatus;

    match patch {
        Some((path, report)) if report.fully_wired() => {
            let inserted = report
                .lines()
                .iter()
                .filter(|(_, _, st)| *st == LineStatus::Inserted)
                .count();
            if inserted == 0 {
                println!(
                    "  2. main.c is already wired (init_rt() / tick_rt() / the include were "
                );
                println!("     already present in {}); just rebuild.", path.display());
            } else {
                println!(
                    "  2. Wired the RT into {}: added {} line(s) inside the USER CODE",
                    path.display(), inserted,
                );
                println!(
                    "     fences (init_rt(), tick_rt(), and the device_registry.h include). \
                     Just rebuild."
                );
            }
        }
        Some((path, report)) => {
            println!(
                "  2. Wired what it could into {}, but add these by hand \u{2014} their",
                path.display(),
            );
            println!("     USER CODE region was not found:");
            for (code, tag, status) in report.lines() {
                if status == LineStatus::RegionMissing {
                    println!("       - `{}` inside `USER CODE BEGIN {}`", code, tag);
                }
            }
            println!("     See INTEGRATION_GUIDE.md for the exact placement.");
        }
        None => {
            println!("  2. From your main.c (or main.cpp), `#include \"rt/device_registry.h\"`, then");
            println!("     call init_rt() after HAL_Init() and MX_*_Init() and tick_rt() repeatedly");
            println!("     from the main loop. (Auto-wiring was skipped: no main.c/main.cpp found");
            println!("     next to the generated sources \u{2014} see INTEGRATION_GUIDE.md.)");
        }
    }
}


/// Look for a single `*.ioc` file directly inside `project_path`.
/// Returns `Some(path)` only when exactly one match is found; zero,
/// multiple, or an unreadable directory all yield `None` so the caller
/// falls back to prompting.
fn find_ioc_in_project(project_path: &Path) -> Option<PathBuf> {
    let entries = std::fs::read_dir(project_path).ok()?;
    let mut matches: Vec<PathBuf> = entries
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            p.is_file()
                && p.extension()
                    .and_then(|s| s.to_str())
                    .map(|s| s.eq_ignore_ascii_case("ioc"))
                    .unwrap_or(false)
        })
        .collect();
    if matches.len() == 1 {
        Some(matches.remove(0))
    } else {
        None
    }
}


fn prompt_for_path(prompt: &str) -> Result<PathBuf, String> {
    print!("{}", prompt);
    std::io::stdout().flush().ok();
    let mut line = String::new();
    std::io::stdin()
        .lock()
        .read_line(&mut line)
        .map_err(|e| format!("Failed to read stdin: {}", e))?;
    let trimmed = line.trim().replace("'", "").replace("\"","");
    if trimmed.is_empty() {
        return Err("Empty path".to_string());
    }
    Ok(PathBuf::from(trimmed))
}


/// Prompt with the given text and read a Y/N answer from stdin. Empty
/// input defaults to `true` (yes). Anything starting with 'n' or 'N'
/// returns `false`; everything else is treated as yes.
fn prompt_yes_no(prompt: &str) -> Result<bool, String> {
    print!("{}", prompt);
    std::io::stdout().flush().ok();
    let mut line = String::new();
    std::io::stdin()
        .lock()
        .read_line(&mut line)
        .map_err(|e| format!("Failed to read stdin: {}", e))?;
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return Ok(true);
    }
    let first = trimmed.chars().next().unwrap();
    Ok(first != 'n' && first != 'N')
}
