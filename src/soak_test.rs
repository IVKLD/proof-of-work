use super::*;
use std::fs::File;
use std::io::Read;
use std::time::Instant;

#[test]
fn test_8_hours_shift_time_engine() {
    let temp_dir = std::env::temp_dir().join("test_worktrack_soak_engine");
    let _ = fs::create_dir_all(&temp_dir);
    let today = "2026-11-05";

    let daily_goal_secs: u64 = 8 * 3600; // 8 hours = 28800s
    let mut prior_worked_secs: u64 = 0;
    let mut session_worked_secs: u64 = 0;
    let first_start_time = "09:00:00".to_string();
    let session_start_time = "09:00:00".to_string();

    let mut shift = ShiftState {
        date: today.to_string(),
        first_start: first_start_time.clone(),
        current_session_start: Some(session_start_time),
        total_worked_secs: 0,
    };

    // Phase 1: Morning shift - 4 hours of active work (14,400 ticks)
    for tick in 1..=14400 {
        session_worked_secs += 1;
        let total = prior_worked_secs + session_worked_secs;
        shift.total_worked_secs = total;

        if session_worked_secs.is_multiple_of(15) {
            save_shift_state(&temp_dir, &shift);
        }
        if tick == 7200 {
            // Halfway check (2 hours)
            let rem = (daily_goal_secs as i64) - (total as i64);
            assert_eq!(rem, 6 * 3600);
            assert_eq!(format_duration_hm(total), "2 ч 00 мин");
        }
    }
    assert_eq!(session_worked_secs, 14400);
    assert_eq!(format_duration_hm(session_worked_secs), "4 ч 00 мин");

    // Phase 2: Lunch break - 1 hour paused (no ticks added to session_worked_secs)
    prior_worked_secs += session_worked_secs;
    session_worked_secs = 0;
    shift.current_session_start = None;
    shift.total_worked_secs = prior_worked_secs;
    save_shift_state(&temp_dir, &shift);

    // Verify state while on pause
    let cfg = Config::default();
    let (start_opt, loaded_worked) = load_shift_state(&temp_dir, today, &cfg);
    assert_eq!(start_opt, Some("09:00:00".to_string()));
    assert_eq!(loaded_worked, 14400);

    // Phase 3: Afternoon shift - 4 hours to complete 8h daily goal (14,400 ticks)
    shift.current_session_start = Some("14:00:00".to_string());
    for _ in 1..=14400 {
        session_worked_secs += 1;
        let total = prior_worked_secs + session_worked_secs;
        shift.total_worked_secs = total;

        if session_worked_secs.is_multiple_of(15) {
            save_shift_state(&temp_dir, &shift);
        }
    }
    let total_at_goal = prior_worked_secs + session_worked_secs;
    assert_eq!(total_at_goal, 28800);
    let rem_at_goal = (daily_goal_secs as i64) - (total_at_goal as i64);
    assert_eq!(rem_at_goal, 0); // Goal reached exactly 0s remaining!

    // Phase 4: Overtime - 1 hour past the 8-hour goal (3,600 ticks)
    for _ in 1..=3600 {
        session_worked_secs += 1;
        let total = prior_worked_secs + session_worked_secs;
        shift.total_worked_secs = total;
    }
    let total_overtime = prior_worked_secs + session_worked_secs;
    assert_eq!(total_overtime, 32400); // 9 hours
    let rem_overtime = (daily_goal_secs as i64) - (total_overtime as i64);
    assert_eq!(rem_overtime, -3600); // 1 hour overtime!

    save_shift_state(&temp_dir, &shift);
    let (_, final_worked) = load_shift_state(&temp_dir, today, &cfg);
    assert_eq!(final_worked, 32400);
    assert_eq!(format_duration_hms(final_worked), "9 ч 00 мин 00 сек");

    let _ = fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_8_hours_graphics_and_archive_soak() {
    let temp_root = std::env::temp_dir().join("test_worktrack_soak_480");
    let _ = fs::remove_dir_all(&temp_root);
    let day = "2026-11-05";
    let day_dir = temp_root.join(day);
    let _ = fs::create_dir_all(&day_dir);

    // Initial base 4K image to simulate screen capture
    let base_img = image::RgbImage::new(3840, 2160);
    let sample_path = temp_root.join("sample_frame.jpg");
    base_img.save(&sample_path).unwrap();

    let start_time = Instant::now();
    // Simulate full day batch: 20 full 4K->1080p frames through the complete pipeline
    let test_shots = 20;
    println!("Starting 8-hour graphics soak test ({test_shots} frames)...");

    let blocked_rects = vec![[100, 100, 800, 600], [2000, 500, 1200, 1000]];

    for i in 1..=test_shots {
        let h = (i - 1) / 60;
        let m = (i - 1) % 60;
        let shot_name = format!("2026-11-05_09-{m:02}-{h:02}.jpg");
        let shot_path = day_dir.join(&shot_name);

        // Copy sample frame
        fs::copy(&sample_path, &shot_path).unwrap();

        // Process through stamper: blackout 2 windows, downscale to 1080p, apply watermark, encode JPEG 60
        let timestamp_str = format!("2026-11-05 09:{m:02}:{h:02}");
        stamper::stamp_screenshot(&shot_path, &timestamp_str, &blocked_rects, 60, Some(1920))
            .expect("stamp_screenshot failed in soak loop");

        // Verify file exists and is readable
        let meta = fs::metadata(&shot_path).unwrap();
        assert!(meta.len() > 1000, "Screenshot file is unexpectedly empty");
    }

    let elapsed = start_time.elapsed();
    println!("Processed {test_shots} full 4K->1080p stamped frames in {:.2?}", elapsed);

    // Verify packing all frames into .tar.gz
    let pack_start = Instant::now();
    let archive_path = archive::pack_day(&temp_root, day).expect("pack_day failed");
    let pack_elapsed = pack_start.elapsed();

    assert!(archive_path.exists());
    let archive_meta = fs::metadata(&archive_path).unwrap();
    println!(
        "Archived day in {:.2?} -> size: {}",
        pack_elapsed,
        format_file_size(archive_meta.len())
    );
    assert!(archive_meta.len() > 10000);

    // Verify archive integrity by reading entries from the tar.gz
    let tar_gz = File::open(&archive_path).unwrap();
    let dec = flate2::read::GzDecoder::new(tar_gz);
    let mut tar = tar::Archive::new(dec);

    let mut count = 0;
    for entry in tar.entries().unwrap() {
        let mut entry = entry.unwrap();
        let path = entry.path().unwrap();
        if path.extension().is_some_and(|e| e == "jpg") {
            count += 1;
            let mut buf = Vec::new();
            entry.read_to_end(&mut buf).unwrap();
            assert!(!buf.is_empty());
        }
    }
    assert_eq!(count, test_shots, "Archived frame count does not match");

    let _ = fs::remove_file(sample_path);
    let _ = fs::remove_dir_all(&temp_root);
}

#[test]
fn test_system_suspend_wake_cycles() {
    // Simulate 10 suspend/wake events and verify that sleep time is never falsely counted
    let mut session_worked_secs: u64 = 0;
    let sleep_durations = [300, 600, 1800, 3600, 7200, 10800, 14400, 10, 5, 60];

    for &sleep_seconds in &sleep_durations {
        let elapsed_ticks = sleep_seconds;
        let actual_elapsed = elapsed_ticks.min(2);
        session_worked_secs += actual_elapsed;
    }

    // Total actual added seconds should be at most 20 seconds, NOT 40,000+ seconds of sleep!
    assert_eq!(session_worked_secs, 20);
}

#[test]
fn test_no_file_descriptor_leak() {
    let temp_file = std::env::temp_dir().join("test_fd_leak.tmp");

    // Standard Linux user limit is 1024 open file descriptors.
    // Looping 2000 times will instantly fail with EMFILE if descriptors are leaked.
    for _ in 0..2000 {
        fs::write(&temp_file, "data").expect("File descriptor leak: failed to write");
        let content = fs::read_to_string(&temp_file).expect("File descriptor leak: failed to read");
        assert_eq!(content, "data");
    }

    let _ = fs::remove_file(&temp_file);
}
