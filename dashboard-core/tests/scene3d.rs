use std::f32::consts::{FRAC_PI_2, PI};

use dashboard_core::daylight::daylight_window;
use dashboard_core::scene3d::{
    FLIP_DURATION_SECONDS, SolarPhase, SplitFlapBoard, SplitFlapSlot, build_lit_box_mesh,
    build_lit_cylinder_mesh, build_lit_icosphere_mesh, build_lit_prism_segment_mesh,
    build_lit_transformed_box_mesh, compute_diorama_light_dir, compute_flap_pose,
    compute_solar_scene, digit_half_segment_boxes, digit_segments, diorama_camera,
    diorama_cloud_centers, diorama_cloud_count, diorama_hill_base_vertices,
    diorama_rain_streak_count, is_solar_arc_segment_lit, shade_triangle_color,
    solar_horizontal_scale, split_flap_camera, split_flap_layout, viewport_rect_to_gl,
};
use jiff::Timestamp;
use jiff::civil::date;
use macroquad::camera::Camera3D;
use macroquad::color::Color;
use macroquad::math::{Mat4, vec3};
use time::OffsetDateTime;

const STOCKHOLM_LAT: f64 = 59.3626;
const STOCKHOLM_LON: f64 = 17.9158;
const STOCKHOLM_TZ: &str = "Europe/Stockholm";

fn offset_from_jiff(ts: Timestamp) -> OffsetDateTime {
    OffsetDateTime::from_unix_timestamp_nanos(ts.as_nanosecond()).expect("valid timestamp")
}

fn shift_offset(ts: Timestamp, delta_seconds: i64) -> OffsetDateTime {
    let shifted = ts.as_second() + delta_seconds;
    OffsetDateTime::from_unix_timestamp(shifted).expect("valid shifted timestamp")
}

fn assert_approx(actual: f32, expected: f32, eps: f32) {
    assert!(
        (actual - expected).abs() <= eps,
        "expected {expected} ± {eps}, got {actual}"
    );
}

#[test]
fn viewport_rect_to_gl_converts_standard_1x_dpi_with_bottom_left_y_flip() {
    let vp = viewport_rect_to_gl(24.0, 40.0, 320.0, 120.0, 600.0, 1.0);
    assert_eq!(vp, Some((24, 440, 320, 120)));

    let top_origin = viewport_rect_to_gl(0.0, 0.0, 800.0, 600.0, 600.0, 1.0);
    assert_eq!(top_origin, Some((0, 0, 800, 600)));
}

#[test]
fn viewport_rect_to_gl_scales_by_2x_hidpi() {
    let vp = viewport_rect_to_gl(10.0, 25.0, 200.0, 80.0, 480.0, 2.0);
    assert_eq!(vp, Some((20, 750, 400, 160)));
}

#[test]
fn viewport_rect_to_gl_rejects_non_positive_or_non_finite_dimensions() {
    assert_eq!(viewport_rect_to_gl(0.0, 0.0, 0.0, 100.0, 600.0, 1.0), None);
    assert_eq!(
        viewport_rect_to_gl(0.0, 0.0, -10.0, 100.0, 600.0, 1.0),
        None
    );
    assert_eq!(viewport_rect_to_gl(0.0, 0.0, 100.0, 0.0, 600.0, 1.0), None);
    assert_eq!(viewport_rect_to_gl(0.0, 0.0, 100.0, -5.0, 600.0, 1.0), None);
    assert_eq!(viewport_rect_to_gl(0.0, 0.0, 100.0, 50.0, 0.0, 1.0), None);
    assert_eq!(
        viewport_rect_to_gl(0.0, 0.0, 100.0, 50.0, -600.0, 1.0),
        None
    );
    assert_eq!(viewport_rect_to_gl(0.0, 0.0, 100.0, 50.0, 600.0, 0.0), None);
    assert_eq!(
        viewport_rect_to_gl(0.0, 0.0, 100.0, 50.0, 600.0, -2.0),
        None
    );
    assert_eq!(
        viewport_rect_to_gl(f32::NAN, 0.0, 100.0, 50.0, 600.0, 1.0),
        None
    );
    assert_eq!(
        viewport_rect_to_gl(0.0, 0.0, f32::INFINITY, 50.0, 600.0, 1.0),
        None
    );
}

#[test]
fn viewport_rect_to_gl_clamps_partially_offscreen_rect_and_rejects_fully_offscreen() {
    let clamped = viewport_rect_to_gl(-10.0, 550.0, 100.0, 100.0, 600.0, 1.0);
    assert_eq!(clamped, Some((0, 0, 90, 50)));

    let offscreen_below = viewport_rect_to_gl(10.0, 650.0, 100.0, 50.0, 600.0, 1.0);
    assert_eq!(offscreen_below, None);

    let offscreen_left = viewport_rect_to_gl(-120.0, 20.0, 100.0, 50.0, 600.0, 1.0);
    assert_eq!(offscreen_left, None);
}

#[test]
fn split_flap_board_starts_settled_with_matching_previous_and_current() {
    let board = SplitFlapBoard::new("14:32:09");
    assert!(!board.is_animating());
    assert_eq!(board.slots().len(), 8);

    let chars: Vec<char> = "14:32:09".chars().collect();
    for (slot, expected) in board.slots().iter().zip(chars) {
        assert_eq!(slot.current, expected);
        assert_eq!(slot.previous, expected);
        assert_approx(slot.progress, 1.0, 1e-6);
    }
}

#[test]
fn split_flap_board_triggers_flip_only_for_changed_slots_and_advances_to_settled() {
    let mut board = SplitFlapBoard::new("14:32:09");
    board.update("14:32:10", 0.016);

    assert!(board.is_animating());
    for unchanged in &board.slots()[..6] {
        assert_approx(unchanged.progress, 1.0, 1e-6);
    }
    assert_eq!(board.slots()[6].previous, '0');
    assert_eq!(board.slots()[6].current, '1');
    assert_approx(board.slots()[6].progress, 0.0, 1e-6);
    assert_eq!(board.slots()[7].previous, '9');
    assert_eq!(board.slots()[7].current, '0');
    assert_approx(board.slots()[7].progress, 0.0, 1e-6);

    board.update("14:32:10", FLIP_DURATION_SECONDS * 0.5);
    assert!(board.is_animating());
    assert_approx(board.slots()[6].progress, 0.5, 1e-4);
    assert_approx(board.slots()[7].progress, 0.5, 1e-4);

    board.update("14:32:10", FLIP_DURATION_SECONDS * 0.5);
    assert!(!board.is_animating());
    assert_approx(board.slots()[6].progress, 1.0, 1e-6);
    assert_approx(board.slots()[7].progress, 1.0, 1e-6);
}

#[test]
fn split_flap_board_clamps_negative_dt_and_handles_length_changes_safely() {
    let mut board = SplitFlapBoard::new("12:34");
    board.update("12:35", 0.016);
    board.update("12:35", -1.0);
    assert_approx(board.slots()[4].progress, 0.0, 1e-6);

    board.update("12:35:00", 0.05);
    assert_eq!(board.slots().len(), 8);
    assert_eq!(board.slots()[5].current, ':');
    assert_eq!(board.slots()[7].current, '0');

    board.update("99", 0.05);
    assert_eq!(board.slots().len(), 2);
    assert_eq!(board.slots()[0].previous, '1');
    assert_eq!(board.slots()[0].current, '9');
    assert_eq!(board.slots()[1].previous, '2');
    assert_eq!(board.slots()[1].current, '9');

    board.update("", 0.05);
    assert!(board.slots().is_empty());
    assert!(!board.is_animating());
}

#[test]
fn compute_flap_pose_uses_cubic_easing_and_correct_upper_lower_transition() {
    let start_slot = SplitFlapSlot {
        current: '5',
        previous: '4',
        progress: 0.0,
    };
    let start = compute_flap_pose(&start_slot);
    assert_approx(start.angle_rad, 0.0, 1e-6);
    assert!(start.is_upper_falling);
    assert_approx(start.lower_shadow_alpha, 0.0, 1e-5);

    let quarter = compute_flap_pose(&SplitFlapSlot {
        current: '5',
        previous: '4',
        progress: 0.25,
    });
    assert!(quarter.is_upper_falling);
    assert_approx(quarter.angle_rad, 0.0625 * PI, 1e-5);

    let mid = compute_flap_pose(&SplitFlapSlot {
        current: '5',
        previous: '4',
        progress: 0.5,
    });
    assert!(!mid.is_upper_falling);
    assert_approx(mid.angle_rad, FRAC_PI_2, 1e-5);

    let three_quarters = compute_flap_pose(&SplitFlapSlot {
        current: '5',
        previous: '4',
        progress: 0.75,
    });
    assert!(!three_quarters.is_upper_falling);
    assert_approx(three_quarters.angle_rad, 0.9375 * PI, 1e-5);

    let end = compute_flap_pose(&SplitFlapSlot {
        current: '5',
        previous: '4',
        progress: 1.0,
    });
    assert!(!end.is_upper_falling);
    assert_approx(end.angle_rad, PI, 1e-5);
    assert_approx(end.lower_shadow_alpha, 0.0, 1e-5);
}

#[test]
fn compute_flap_pose_peaks_lower_shadow_mid_flip() {
    let early = compute_flap_pose(&SplitFlapSlot {
        current: '8',
        previous: '7',
        progress: 0.1,
    });
    let late = compute_flap_pose(&SplitFlapSlot {
        current: '8',
        previous: '7',
        progress: 0.9,
    });
    let mid = compute_flap_pose(&SplitFlapSlot {
        current: '8',
        previous: '7',
        progress: 0.5,
    });
    assert!(mid.lower_shadow_alpha > early.lower_shadow_alpha);
    assert!(mid.lower_shadow_alpha > late.lower_shadow_alpha);
    assert!(mid.lower_shadow_alpha > 0.2);
}

#[test]
fn compute_solar_scene_produces_25_symmetric_arc_points_from_east_to_west() {
    let window = daylight_window(
        STOCKHOLM_TZ,
        STOCKHOLM_LAT,
        STOCKHOLM_LON,
        date(2026, 6, 21),
    )
    .expect("daylight window");
    let noon_secs = (window.sunrise_utc.as_second() + window.sunset_utc.as_second()) / 2;
    let noon = OffsetDateTime::from_unix_timestamp(noon_secs).expect("valid noon");

    let scene = compute_solar_scene(&window, noon, STOCKHOLM_TZ);
    assert_eq!(scene.arc_points.len(), 25);
    assert_approx(scene.arc_points[0][0], -3.2, 1e-4);
    assert_approx(scene.arc_points[0][1], 0.0, 1e-4);
    assert_approx(scene.arc_points[12][0], 0.0, 1e-4);
    assert_approx(scene.arc_points[12][1], 1.7, 1e-3);
    assert_approx(scene.arc_points[24][0], 3.2, 1e-4);
    assert_approx(scene.arc_points[24][1], 0.0, 1e-4);

    for pair in scene.arc_points.windows(2) {
        assert!(pair[1][0] > pair[0][0]);
    }
    assert!(scene.arc_points[12][2] > scene.arc_points[0][2]);
    assert_approx(scene.arc_points[0][2], scene.arc_points[24][2], 1e-5);
}

#[test]
fn compute_solar_scene_classifies_all_five_solar_phases_and_distinct_palettes() {
    let window = daylight_window(
        STOCKHOLM_TZ,
        STOCKHOLM_LAT,
        STOCKHOLM_LON,
        date(2026, 3, 21),
    )
    .expect("equinox daylight window");

    let dawn_ts = window
        .civil_dawn
        .in_tz(STOCKHOLM_TZ)
        .expect("dawn tz")
        .timestamp();
    let dusk_ts = window
        .civil_dusk
        .in_tz(STOCKHOLM_TZ)
        .expect("dusk tz")
        .timestamp();
    let day_len = window.sunset_utc.as_second() - window.sunrise_utc.as_second();

    let night = compute_solar_scene(&window, shift_offset(dawn_ts, -3600), STOCKHOLM_TZ);
    let civil_dawn = compute_solar_scene(
        &window,
        offset_from_jiff(
            Timestamp::from_second((dawn_ts.as_second() + window.sunrise_utc.as_second()) / 2)
                .unwrap(),
        ),
        STOCKHOLM_TZ,
    );
    let morning_golden = compute_solar_scene(
        &window,
        shift_offset(window.sunrise_utc, day_len / 20),
        STOCKHOLM_TZ,
    );
    let midday = compute_solar_scene(
        &window,
        shift_offset(window.sunrise_utc, day_len / 2),
        STOCKHOLM_TZ,
    );
    let evening_golden = compute_solar_scene(
        &window,
        shift_offset(window.sunset_utc, -day_len / 20),
        STOCKHOLM_TZ,
    );
    let civil_dusk = compute_solar_scene(
        &window,
        offset_from_jiff(
            Timestamp::from_second((window.sunset_utc.as_second() + dusk_ts.as_second()) / 2)
                .unwrap(),
        ),
        STOCKHOLM_TZ,
    );
    let late_night = compute_solar_scene(&window, shift_offset(dusk_ts, 3600), STOCKHOLM_TZ);

    assert_eq!(night.phase, SolarPhase::Night);
    assert_eq!(civil_dawn.phase, SolarPhase::CivilDawn);
    assert_eq!(morning_golden.phase, SolarPhase::GoldenHour);
    assert_eq!(midday.phase, SolarPhase::Day);
    assert_eq!(evening_golden.phase, SolarPhase::GoldenHour);
    assert_eq!(civil_dusk.phase, SolarPhase::CivilDusk);
    assert_eq!(late_night.phase, SolarPhase::Night);

    assert_ne!(night.sky_tint, midday.sky_tint);
    assert_ne!(midday.sun_color, morning_golden.sun_color);
    assert_ne!(civil_dawn.horizon_glow, civil_dusk.horizon_glow);
}

#[test]
fn compute_solar_scene_positions_sun_at_noon_peak_and_below_horizon_at_twilight_and_night() {
    let window = daylight_window(
        STOCKHOLM_TZ,
        STOCKHOLM_LAT,
        STOCKHOLM_LON,
        date(2026, 3, 21),
    )
    .expect("equinox daylight window");

    let dawn_ts = window
        .civil_dawn
        .in_tz(STOCKHOLM_TZ)
        .expect("dawn tz")
        .timestamp();
    let dusk_ts = window
        .civil_dusk
        .in_tz(STOCKHOLM_TZ)
        .expect("dusk tz")
        .timestamp();
    let noon_secs = (window.sunrise_utc.as_second() + window.sunset_utc.as_second()) / 2;

    let noon = compute_solar_scene(
        &window,
        OffsetDateTime::from_unix_timestamp(noon_secs).unwrap(),
        STOCKHOLM_TZ,
    );
    assert_approx(noon.daylight_fraction, 0.5, 1e-3);
    assert_approx(noon.sun_altitude_norm, 1.0, 1e-3);
    assert_approx(noon.sun_position[0], 0.0, 1e-2);
    assert_approx(noon.sun_position[1], 1.7, 1e-2);

    let dawn = compute_solar_scene(
        &window,
        offset_from_jiff(
            Timestamp::from_second((dawn_ts.as_second() + window.sunrise_utc.as_second()) / 2)
                .unwrap(),
        ),
        STOCKHOLM_TZ,
    );
    assert!(dawn.sun_position[1] < 0.0);
    assert!(dawn.sun_altitude_norm < 0.0);
    assert!(dawn.sun_position[0] < 0.0);

    let dusk = compute_solar_scene(
        &window,
        offset_from_jiff(
            Timestamp::from_second((window.sunset_utc.as_second() + dusk_ts.as_second()) / 2)
                .unwrap(),
        ),
        STOCKHOLM_TZ,
    );
    assert!(dusk.sun_position[1] < 0.0);
    assert!(dusk.sun_altitude_norm < 0.0);
    assert!(dusk.sun_position[0] > 0.0);

    let deep_night = compute_solar_scene(&window, shift_offset(dusk_ts, 7200), STOCKHOLM_TZ);
    assert!(deep_night.sun_position[1] < dusk.sun_position[1]);
}

#[test]
fn compute_solar_scene_clamps_daylight_fraction_before_sunrise_and_after_sunset() {
    let window = daylight_window(
        STOCKHOLM_TZ,
        STOCKHOLM_LAT,
        STOCKHOLM_LON,
        date(2026, 11, 24),
    )
    .expect("november daylight window");

    let before = compute_solar_scene(
        &window,
        shift_offset(window.sunrise_utc, -3600),
        STOCKHOLM_TZ,
    );
    assert_approx(before.daylight_fraction, 0.0, 1e-6);

    let after = compute_solar_scene(&window, shift_offset(window.sunset_utc, 3600), STOCKHOLM_TZ);
    assert_approx(after.daylight_fraction, 1.0, 1e-6);
}

fn camera_view_proj(cam: &Camera3D) -> Mat4 {
    let aspect = cam.aspect.expect("camera must set explicit aspect");
    Mat4::perspective_rh_gl(cam.fovy, aspect, cam.z_near, cam.z_far)
        * Mat4::look_at_rh(cam.position, cam.target, cam.up)
}

#[test]
fn diorama_camera_fits_entire_pedestal_pylons_and_noon_sun_inside_ndc_with_wide_coverage() {
    let window = daylight_window(
        STOCKHOLM_TZ,
        STOCKHOLM_LAT,
        STOCKHOLM_LON,
        date(2026, 6, 21),
    )
    .expect("daylight window");
    let noon_secs = (window.sunrise_utc.as_second() + window.sunset_utc.as_second()) / 2;
    let scene = compute_solar_scene(
        &window,
        OffsetDateTime::from_unix_timestamp(noon_secs).unwrap(),
        STOCKHOLM_TZ,
    );

    let orbit_samples = [0.0_f32, FRAC_PI_2 / 0.25, -FRAC_PI_2 / 0.25, PI / 0.25];

    for (width, height) in [(1160.0_f32, 174.0_f32), (1880.0_f32, 174.0_f32)] {
        let x_scale = solar_horizontal_scale(width, height);
        assert!(x_scale > 2.5);

        let plinth_rx = 3.94 * x_scale;
        let plinth_rz = 2.26;
        let east_x = scene.arc_points[0][0] * x_scale;
        let west_x = scene.arc_points[scene.arc_points.len() - 1][0] * x_scale;
        let arc_z = scene.arc_points[0][2] + 1.0;
        let noon_z = scene.sun_position[2] + 1.0;

        let test_points = [
            ("front_plinth_bottom", vec3(0.0, -0.46, plinth_rz)),
            ("back_plinth_top", vec3(0.0, 0.0, -plinth_rz)),
            ("left_plinth_bottom", vec3(-plinth_rx, -0.46, 0.0)),
            ("right_plinth_bottom", vec3(plinth_rx, -0.46, 0.0)),
            ("east_pylon_top", vec3(east_x, 0.54, arc_z)),
            ("west_pylon_top", vec3(west_x, 0.54, arc_z)),
            (
                "noon_sun_armillary_top",
                vec3(0.0, scene.sun_position[1] + 0.56, noon_z),
            ),
        ];

        for orbit_t in orbit_samples {
            let cam = diorama_camera(width, height, orbit_t);
            let vp = camera_view_proj(&cam);

            for (label, pt) in test_points {
                let ndc = vp.project_point3(pt);
                assert!(
                    ndc.x.abs() <= 0.95 && ndc.y.abs() <= 0.95,
                    "{width}x{height} t={orbit_t:.2} {label} clipped: ndc=({:.3}, {:.3})",
                    ndc.x,
                    ndc.y
                );
            }

            let ndc_left = vp.project_point3(vec3(-plinth_rx, 0.0, 0.0));
            let ndc_right = vp.project_point3(vec3(plinth_rx, 0.0, 0.0));
            let coverage = (ndc_right.x - ndc_left.x) * 0.5;
            assert!(
                coverage >= 0.60,
                "{width}x{height} t={orbit_t:.2} expected >= 60% width coverage, got {:.1}%",
                coverage * 100.0
            );
        }
    }

    assert_approx(solar_horizontal_scale(80.0, 80.0), 1.0, 1e-5);
}

#[test]
fn split_flap_camera_fits_outer_casings_and_mid_flip_flaps_inside_ndc_across_sway() {
    let slot_count = 8;
    let sway_times = [0.0_f32, FRAC_PI_2 / 0.45, -FRAC_PI_2 / 0.45];

    for (width, height) in [(300.0_f32, 80.0_f32), (680.0_f32, 190.0_f32)] {
        let layout = split_flap_layout(width, height, slot_count);
        let outer_w = layout.tile_half_w + layout.wall_t;
        let outer_h = layout.tile_half_h + layout.wall_t;

        for t in sway_times {
            let cam = split_flap_camera(width, height, t);
            let vp = camera_view_proj(&cam);

            for slot_idx in [0usize, slot_count - 1] {
                let cx = layout.start_x + (slot_idx as f32) * layout.slot_pitch_x;
                let probe_points = [
                    ("casing_front_tl", vec3(cx - outer_w, outer_h, 0.052)),
                    ("casing_front_tr", vec3(cx + outer_w, outer_h, 0.052)),
                    ("casing_front_bl", vec3(cx - outer_w, -outer_h, 0.052)),
                    ("casing_front_br", vec3(cx + outer_w, -outer_h, 0.052)),
                    ("casing_back_tl", vec3(cx - outer_w, outer_h, -0.23)),
                    ("casing_back_br", vec3(cx + outer_w, -outer_h, -0.23)),
                    (
                        "mid_flip_left_tip",
                        vec3(cx - layout.tile_half_w, 0.0, 0.018 + layout.tile_half_h),
                    ),
                    (
                        "mid_flip_right_tip",
                        vec3(cx + layout.tile_half_w, 0.0, 0.018 + layout.tile_half_h),
                    ),
                ];

                for (label, pt) in probe_points {
                    let ndc = vp.project_point3(pt);
                    assert!(
                        ndc.x.abs() <= 0.96 && ndc.y.abs() <= 0.96,
                        "{width}x{height} slot={slot_idx} t={t:.2} {label} clipped: ndc=({:.3}, {:.3})",
                        ndc.x,
                        ndc.y
                    );
                }
            }
        }
    }
}

#[test]
fn diorama_hill_bases_stay_inside_elliptical_pedestal_across_aspect_scales() {
    let pedestal_rz = 2.15_f32;
    for x_scale in [1.0_f32, 3.33, 5.40] {
        let pedestal_rx = 3.8 * x_scale;
        let vertices = diorama_hill_base_vertices(x_scale);
        assert!(!vertices.is_empty());
        for v in vertices {
            let norm = (v.x / pedestal_rx).powi(2) + (v.z / pedestal_rz).powi(2);
            assert!(
                norm <= 1.0,
                "x_scale={x_scale} hill base vertex {:?} overhangs pedestal ellipse (norm={norm:.3})",
                v
            );
        }
    }
}

#[test]
fn diorama_clouds_stay_clear_of_noon_sun_and_armillary_volume() {
    let noon_sun = vec3(0.0, 1.7, 0.2);
    let min_clearance = 0.56 + 0.34;
    let pedestal_rz = 2.15_f32;
    let shadow_radius = 0.52_f32;

    for x_scale in [1.0_f32, 3.33, 5.40] {
        let pedestal_rx = 3.8 * x_scale;
        for step in 0..24 {
            let t = step as f32 * 1.5;
            let clouds = diorama_cloud_centers(85, x_scale, t);
            assert_eq!(clouds.len(), 3);
            for cloud in clouds {
                let dist = (cloud - noon_sun).length();
                assert!(
                    dist > min_clearance,
                    "cloud at {:?} overlapped noon sun/armillary volume (dist={dist:.3} <= {min_clearance:.3})",
                    cloud
                );
                for sample in 0..16 {
                    let angle = (sample as f32 / 16.0) * std::f32::consts::TAU;
                    let sx = cloud.x + shadow_radius * angle.cos();
                    let sz = cloud.z + shadow_radius * angle.sin();
                    let norm = (sx / pedestal_rx).powi(2) + (sz / pedestal_rz).powi(2);
                    assert!(
                        norm <= 1.0,
                        "x_scale={x_scale} t={t:.1} cloud shadow at ({sx:.2}, {sz:.2}) overhangs pedestal (norm={norm:.3})"
                    );
                }
            }
        }
    }
}

#[test]
fn compute_solar_scene_handles_dst_transition_dates_cleanly() {
    for transition_date in [date(2026, 3, 29), date(2026, 10, 25)] {
        let window = daylight_window(STOCKHOLM_TZ, STOCKHOLM_LAT, STOCKHOLM_LON, transition_date)
            .expect("dst transition daylight window");
        let dawn_ts = window
            .civil_dawn
            .in_tz(STOCKHOLM_TZ)
            .expect("dawn tz")
            .timestamp();
        let dusk_ts = window
            .civil_dusk
            .in_tz(STOCKHOLM_TZ)
            .expect("dusk tz")
            .timestamp();
        let noon_secs = (window.sunrise_utc.as_second() + window.sunset_utc.as_second()) / 2;

        let dawn = compute_solar_scene(
            &window,
            offset_from_jiff(
                Timestamp::from_second((dawn_ts.as_second() + window.sunrise_utc.as_second()) / 2)
                    .unwrap(),
            ),
            STOCKHOLM_TZ,
        );
        let noon = compute_solar_scene(
            &window,
            OffsetDateTime::from_unix_timestamp(noon_secs).unwrap(),
            STOCKHOLM_TZ,
        );
        let dusk = compute_solar_scene(
            &window,
            offset_from_jiff(
                Timestamp::from_second((window.sunset_utc.as_second() + dusk_ts.as_second()) / 2)
                    .unwrap(),
            ),
            STOCKHOLM_TZ,
        );

        assert_eq!(dawn.phase, SolarPhase::CivilDawn);
        assert_eq!(noon.phase, SolarPhase::Day);
        assert_eq!(dusk.phase, SolarPhase::CivilDusk);
        assert!(noon.sun_position.iter().all(|v| v.is_finite()));
        assert!(noon.sun_position[1] > 1.5);
    }
}

#[test]
fn compute_solar_scene_falls_back_gracefully_for_invalid_time_zone() {
    let window = daylight_window(
        STOCKHOLM_TZ,
        STOCKHOLM_LAT,
        STOCKHOLM_LON,
        date(2026, 3, 21),
    )
    .expect("equinox daylight window");
    let noon_secs = (window.sunrise_utc.as_second() + window.sunset_utc.as_second()) / 2;

    let dawn_fallback = compute_solar_scene(
        &window,
        shift_offset(window.sunrise_utc, -600),
        "Invalid/Timezone",
    );
    let noon_fallback = compute_solar_scene(
        &window,
        OffsetDateTime::from_unix_timestamp(noon_secs).unwrap(),
        "Invalid/Timezone",
    );
    let dusk_fallback = compute_solar_scene(
        &window,
        shift_offset(window.sunset_utc, 600),
        "Invalid/Timezone",
    );

    assert_eq!(dawn_fallback.phase, SolarPhase::CivilDawn);
    assert_eq!(noon_fallback.phase, SolarPhase::Day);
    assert_eq!(dusk_fallback.phase, SolarPhase::CivilDusk);
}

#[test]
fn compute_solar_scene_keeps_traversed_arc_lit_at_exact_sunset_and_unlit_at_sunrise_and_twilight() {
    let window = daylight_window(
        STOCKHOLM_TZ,
        STOCKHOLM_LAT,
        STOCKHOLM_LON,
        date(2026, 6, 21),
    )
    .expect("daylight window");

    let at_sunrise =
        compute_solar_scene(&window, offset_from_jiff(window.sunrise_utc), STOCKHOLM_TZ);
    assert_eq!(at_sunrise.phase, SolarPhase::GoldenHour);
    assert_approx(at_sunrise.daylight_fraction, 0.0, 1e-6);
    assert!(!is_solar_arc_segment_lit(&at_sunrise, 0.05));

    let at_sunset = compute_solar_scene(&window, offset_from_jiff(window.sunset_utc), STOCKHOLM_TZ);
    assert_eq!(at_sunset.phase, SolarPhase::GoldenHour);
    assert_approx(at_sunset.daylight_fraction, 1.0, 1e-6);
    assert_approx(at_sunset.sun_altitude_norm, 0.0, 1e-5);
    assert_approx(at_sunset.sun_position[0], 3.2, 1e-4);
    assert_approx(at_sunset.sun_position[1], 0.0, 1e-4);
    assert!(is_solar_arc_segment_lit(&at_sunset, 0.5));
    assert!(is_solar_arc_segment_lit(&at_sunset, 0.99));

    let at_dusk = compute_solar_scene(&window, shift_offset(window.sunset_utc, 600), STOCKHOLM_TZ);
    assert_eq!(at_dusk.phase, SolarPhase::CivilDusk);
    assert!(!is_solar_arc_segment_lit(&at_dusk, 0.5));
}

fn color_luminance(c: Color) -> f32 {
    0.2126 * c.r + 0.7152 * c.g + 0.0722 * c.b
}

#[test]
fn shade_triangle_color_is_strictly_brighter_for_light_facing_than_side_and_back_normals() {
    let base = Color::new(0.4, 0.6, 0.8, 0.85);
    let light_dir = vec3(0.0, 1.0, 0.0);
    let view_dir = vec3(0.0, 1.0, 0.0);

    let front = shade_triangle_color(base, vec3(0.0, 1.0, 0.0), light_dir, view_dir, 0.25, 0.35);
    let front_no_spec =
        shade_triangle_color(base, vec3(0.0, 1.0, 0.0), light_dir, view_dir, 0.25, 0.0);
    let side = shade_triangle_color(base, vec3(1.0, 0.0, 0.0), light_dir, view_dir, 0.25, 0.35);
    let back = shade_triangle_color(base, vec3(0.0, -1.0, 0.0), light_dir, view_dir, 0.25, 0.35);

    assert!(color_luminance(front) > color_luminance(front_no_spec));
    assert!(color_luminance(front_no_spec) > color_luminance(side));
    assert!(color_luminance(side) > color_luminance(back));
    assert_approx(front.a, 0.85, 1e-6);
    assert_approx(back.a, 0.85, 1e-6);
}

#[test]
fn build_lit_box_mesh_has_24_vertices_36_indices_and_distinct_lit_face_colors() {
    let center = vec3(1.0, -0.5, 0.25);
    let half_extents = vec3(0.4, 0.6, 0.2);
    let base = Color::new(0.35, 0.50, 0.70, 1.0);
    let light_dir = vec3(0.3, 0.85, 0.45).normalize();
    let view_dir = vec3(0.0, 0.3, 1.0).normalize();

    let mesh = build_lit_box_mesh(center, half_extents, base, light_dir, view_dir);
    assert_eq!(mesh.vertices.len(), 24);
    assert_eq!(mesh.indices.len(), 36);

    for v in &mesh.vertices {
        assert!(v.position.x >= center.x - half_extents.x - 1e-4);
        assert!(v.position.x <= center.x + half_extents.x + 1e-4);
        assert!(v.position.y >= center.y - half_extents.y - 1e-4);
        assert!(v.position.y <= center.y + half_extents.y + 1e-4);
        assert!(v.position.z >= center.z - half_extents.z - 1e-4);
        assert!(v.position.z <= center.z + half_extents.z + 1e-4);
    }

    let mut unique_colors = std::collections::HashSet::new();
    for v in &mesh.vertices {
        unique_colors.insert(v.color);
    }
    assert!(
        unique_colors.len() >= 4,
        "expected distinct shaded face colors, got {}",
        unique_colors.len()
    );
}

#[test]
fn build_lit_transformed_box_mesh_rotates_positions_and_face_lighting_around_hinge() {
    let origin = vec3(0.5, 0.0, 0.02);
    let pivot_offset = vec3(0.0, 0.25, 0.0);
    let half_extents = vec3(0.2, 0.25, 0.015);
    let base = Color::new(0.25, 0.35, 0.55, 1.0);
    let light_dir = vec3(0.0, 0.9, 0.43).normalize();
    let view_dir = vec3(0.0, 0.2, 1.0).normalize();

    let upright = build_lit_transformed_box_mesh(
        origin,
        half_extents,
        0.0,
        pivot_offset,
        base,
        light_dir,
        view_dir,
    );
    let horizontal = build_lit_transformed_box_mesh(
        origin,
        half_extents,
        FRAC_PI_2,
        pivot_offset,
        base,
        light_dir,
        view_dir,
    );
    let flipped = build_lit_transformed_box_mesh(
        origin,
        half_extents,
        PI,
        pivot_offset,
        base,
        light_dir,
        view_dir,
    );

    assert_eq!(upright.vertices.len(), 24);
    assert_eq!(horizontal.vertices.len(), 24);
    assert_eq!(flipped.vertices.len(), 24);

    let max_y_upright = upright
        .vertices
        .iter()
        .map(|v| v.position.y)
        .fold(f32::NEG_INFINITY, f32::max);
    let max_z_horizontal = horizontal
        .vertices
        .iter()
        .map(|v| v.position.z)
        .fold(f32::NEG_INFINITY, f32::max);
    let min_y_flipped = flipped
        .vertices
        .iter()
        .map(|v| v.position.y)
        .fold(f32::INFINITY, f32::min);

    assert_approx(max_y_upright, 0.5, 1e-4);
    assert_approx(max_z_horizontal, origin.z + 0.5, 1e-4);
    assert_approx(min_y_flipped, -0.5, 1e-4);

    let upright_colors: Vec<[u8; 4]> = upright.vertices.iter().map(|v| v.color).collect();
    let horizontal_colors: Vec<[u8; 4]> = horizontal.vertices.iter().map(|v| v.color).collect();
    assert_ne!(upright_colors, horizontal_colors);
}

#[test]
fn build_lit_icosphere_mesh_places_vertices_at_exact_radius_with_non_uniform_lighting() {
    let center = vec3(-0.7, 1.2, 0.3);
    let radius = 0.42;
    let base = Color::new(0.95, 0.72, 0.30, 1.0);
    let light_dir = vec3(0.5, 0.8, 0.3).normalize();
    let view_dir = vec3(0.0, 0.3, 1.0).normalize();

    let coarse = build_lit_icosphere_mesh(center, radius, 0, base, light_dir, view_dir, 0.0);
    assert_eq!(coarse.vertices.len(), 60);
    assert_eq!(coarse.indices.len(), 60);

    let detailed = build_lit_icosphere_mesh(center, radius, 2, base, light_dir, view_dir, 0.35);
    assert_eq!(detailed.vertices.len(), 960);
    assert_eq!(detailed.indices.len(), 960);

    let mut unique_colors = std::collections::HashSet::new();
    for v in &detailed.vertices {
        let dist = (v.position - center).length();
        assert_approx(dist, radius, 1e-4);
        unique_colors.insert(v.color);
    }
    assert!(
        unique_colors.len() >= 12,
        "expected rich faceted lighting across icosphere, got {} distinct colors",
        unique_colors.len()
    );
}

#[test]
fn build_lit_prism_segment_mesh_creates_non_degenerate_3d_volume_around_axis() {
    let start = vec3(-1.0, 0.2, -0.4);
    let end = vec3(1.2, 1.4, 0.6);
    let radius = 0.08;
    let base = Color::new(0.85, 0.60, 0.25, 1.0);
    let light_dir = vec3(0.2, 0.9, 0.3).normalize();
    let view_dir = vec3(0.0, 0.2, 1.0).normalize();

    let mesh = build_lit_prism_segment_mesh(start, end, radius, base, light_dir, view_dir);
    assert!(!mesh.vertices.is_empty());
    assert_eq!(mesh.indices.len() % 3, 0);

    let axis = (end - start).normalize();
    let mut saw_rim_radius = false;
    let mut unique_colors = std::collections::HashSet::new();
    for v in &mesh.vertices {
        let rel = v.position - start;
        let along = rel.dot(axis);
        let perp = (rel - axis * along).length();
        assert!(perp <= radius + 1e-4);
        if (perp - radius).abs() < 1e-3 {
            saw_rim_radius = true;
        }
        unique_colors.insert(v.color);
    }
    assert!(saw_rim_radius);
    assert!(unique_colors.len() >= 3);

    let degenerate = build_lit_prism_segment_mesh(start, start, radius, base, light_dir, view_dir);
    assert!(degenerate.vertices.is_empty());
    assert!(degenerate.indices.is_empty());
}

#[test]
fn build_lit_cylinder_mesh_builds_top_bottom_and_shaded_sides() {
    let center = vec3(0.0, -0.2, 0.0);
    let radius = 2.5;
    let height = 0.4;
    let top_color = Color::new(0.22, 0.34, 0.50, 1.0);
    let side_color = Color::new(0.10, 0.14, 0.22, 1.0);
    let light_dir = vec3(0.4, 0.8, 0.45).normalize();
    let view_dir = vec3(0.0, 0.5, 1.0).normalize();

    let mesh = build_lit_cylinder_mesh(
        center, radius, height, 16, top_color, side_color, light_dir, view_dir,
    );
    assert!(!mesh.vertices.is_empty());
    assert_eq!(mesh.indices.len() % 3, 0);

    let mut min_y = f32::INFINITY;
    let mut max_y = f32::NEG_INFINITY;
    let mut unique_colors = std::collections::HashSet::new();
    for v in &mesh.vertices {
        min_y = min_y.min(v.position.y);
        max_y = max_y.max(v.position.y);
        let radial = (v.position.x * v.position.x + v.position.z * v.position.z).sqrt();
        assert!(radial <= radius + 1e-4);
        unique_colors.insert(v.color);
    }
    assert_approx(min_y, center.y - height * 0.5, 1e-4);
    assert_approx(max_y, center.y + height * 0.5, 1e-4);
    assert!(unique_colors.len() >= 5);

    let empty = build_lit_cylinder_mesh(
        center, 0.0, height, 16, top_color, side_color, light_dir, view_dir,
    );
    assert!(empty.vertices.is_empty());

    let bounded = build_lit_cylinder_mesh(
        center, radius, height, 8000, top_color, side_color, light_dir, view_dir,
    );
    assert!(bounded.vertices.len() <= usize::from(u16::MAX));
    assert!(
        bounded
            .indices
            .iter()
            .all(|&idx| usize::from(idx) < bounded.vertices.len())
    );
}

#[test]
fn digit_segments_maps_all_decimal_digits_and_partitions_cleanly_across_hinge() {
    assert_eq!(
        digit_segments('0'),
        [true, true, true, false, true, true, true]
    );
    assert_eq!(
        digit_segments('1'),
        [false, false, true, false, false, true, false]
    );
    assert_eq!(
        digit_segments('2'),
        [true, false, true, true, true, false, true]
    );
    assert_eq!(
        digit_segments('3'),
        [true, false, true, true, false, true, true]
    );
    assert_eq!(
        digit_segments('4'),
        [false, true, true, true, false, true, false]
    );
    assert_eq!(
        digit_segments('5'),
        [true, true, false, true, false, true, true]
    );
    assert_eq!(
        digit_segments('6'),
        [true, true, false, true, true, true, true]
    );
    assert_eq!(
        digit_segments('7'),
        [true, false, true, false, false, true, false]
    );
    assert_eq!(
        digit_segments('8'),
        [true, true, true, true, true, true, true]
    );
    assert_eq!(
        digit_segments('9'),
        [true, true, true, true, false, true, true]
    );
    assert_eq!(digit_segments(':'), [false; 7]);

    let upper_eight = digit_half_segment_boxes('8', true, 0.28, 0.44);
    let lower_eight = digit_half_segment_boxes('8', false, 0.28, 0.44);
    assert_eq!(upper_eight.len(), 4);
    assert_eq!(lower_eight.len(), 4);
    for (center, half) in &upper_eight {
        assert!(center.y - half.y >= 0.0);
        assert!(half.z > 0.0);
    }
    for (center, half) in &lower_eight {
        assert!(center.y + half.y <= 0.0);
        assert!(half.z > 0.0);
    }

    let upper_colon = digit_half_segment_boxes(':', true, 0.28, 0.44);
    let lower_colon = digit_half_segment_boxes(':', false, 0.28, 0.44);
    assert_eq!(upper_colon.len(), 1);
    assert_eq!(lower_colon.len(), 1);
    assert!(upper_colon[0].0.y > 0.0);
    assert!(lower_colon[0].0.y < 0.0);
}

#[test]
fn diorama_weather_and_lighting_helpers_scale_clouds_rain_and_sun_direction() {
    assert_eq!(diorama_cloud_count(0), 0);
    assert_eq!(diorama_cloud_count(9), 0);
    assert_eq!(diorama_cloud_count(10), 2);
    assert_eq!(diorama_cloud_count(35), 2);
    assert_eq!(diorama_cloud_count(40), 3);
    assert_eq!(diorama_cloud_count(100), 3);

    assert_eq!(diorama_rain_streak_count(0), 0);
    assert_eq!(diorama_rain_streak_count(19), 0);
    let light_rain = diorama_rain_streak_count(20);
    let medium_rain = diorama_rain_streak_count(60);
    let heavy_rain = diorama_rain_streak_count(100);
    assert!(light_rain >= 4);
    assert!(medium_rain > light_rain);
    assert!(heavy_rain > medium_rain);

    let window = daylight_window(
        STOCKHOLM_TZ,
        STOCKHOLM_LAT,
        STOCKHOLM_LON,
        date(2026, 6, 21),
    )
    .expect("daylight window");
    let day_len = window.sunset_utc.as_second() - window.sunrise_utc.as_second();
    let morning = compute_solar_scene(
        &window,
        shift_offset(window.sunrise_utc, day_len / 4),
        STOCKHOLM_TZ,
    );
    let afternoon = compute_solar_scene(
        &window,
        shift_offset(window.sunrise_utc, (day_len * 3) / 4),
        STOCKHOLM_TZ,
    );
    let night = compute_solar_scene(
        &window,
        shift_offset(window.sunrise_utc, -10_800),
        STOCKHOLM_TZ,
    );

    let morning_dir = compute_diorama_light_dir(&morning);
    let afternoon_dir = compute_diorama_light_dir(&afternoon);
    let night_dir = compute_diorama_light_dir(&night);

    assert_approx(morning_dir.length(), 1.0, 1e-4);
    assert_approx(afternoon_dir.length(), 1.0, 1e-4);
    assert_approx(night_dir.length(), 1.0, 1e-4);
    assert!(morning_dir.x < 0.0);
    assert!(afternoon_dir.x > 0.0);
    assert!(morning_dir.y > 0.0);
    assert!(night_dir.y > 0.0);
}
