//! Camera rig: advances `s` along the closed flight loop and produces the per-frame pose
//! (position, view direction, rolled up vector, fixed 58-degree vertical FOV), including the
//! legacy low-passed bank-on-yaw algorithm ported from `GibsonSCNCamera.mm` (see
//! `docs/legacy-banking.md`).

use gibson_types::{CameraPose, Settings};
use glam::Vec3;

use crate::FlightPath;

/// Fixed vertical field of view (legacy camera).
const FOV_Y_RADIANS: f32 = 58f32.to_radians();
/// Tangent sample spacing, in segments (legacy `eps = 0.14`).
const TANGENT_EPS: f32 = 0.14;
/// Yaw-rate low-pass time constant, seconds (legacy 0.28).
const YAW_TAU: f32 = 0.28;
/// Look-ahead along the path, seconds (legacy `GIBSON_FLY_LOOKAHEAD_MS 150`).
const LOOKAHEAD_SECONDS: f32 = 0.15;
/// `signedYaw` magnitudes above this are treated as wrap/seam artifacts and zeroed (legacy 0.45).
const YAW_ARTIFACT_THRESHOLD: f32 = 0.45;

/// Camera state machine: current path position `s` plus the two smoothed banking states.
pub(crate) struct CameraRig {
    /// Position along the closed loop, in segments, kept wrapped in `[0, len)` for f32 precision.
    s: f32,
    /// Low-passed signed yaw rate (radians per segment).
    yaw_rate: f32,
    /// Smoothed roll angle about the forward axis, radians.
    bank: f32,
    pose: CameraPose,
    prev_pose: CameraPose,
    /// True once `advance` has run at least once (with `dt == 0` on the first call).
    started: bool,
}

impl CameraRig {
    /// Rig parked at the start of the loop: pose at `s = 0`, zero bank, `prev == current`.
    pub(crate) fn new(path: &FlightPath, fly_speed: f32) -> CameraRig {
        let mut rig = CameraRig {
            s: 0.0,
            yaw_rate: 0.0,
            bank: 0.0,
            pose: CameraPose {
                position: [0.0, 0.0, 0.0],
                forward: [0.0, 0.0, -1.0],
                up: [0.0, 1.0, 0.0],
                fov_y_radians: FOV_Y_RADIANS,
            },
            prev_pose: CameraPose {
                position: [0.0, 0.0, 0.0],
                forward: [0.0, 0.0, -1.0],
                up: [0.0, 1.0, 0.0],
                fov_y_radians: FOV_Y_RADIANS,
            },
            started: false,
        };
        let pose = rig.pose_at(path, fly_speed);
        rig.pose = pose;
        rig.prev_pose = pose;
        rig
    }

    fn pose_at(&self, path: &FlightPath, fly_speed: f32) -> CameraPose {
        let pos = Vec3::from_array(path.position(self.s));
        let look = Vec3::from_array(path.position(self.s + fly_speed * LOOKAHEAD_SECONDS));
        let fallback = Vec3::from_array(self.pose.forward);
        let forward = safe_norm(look - pos, fallback);
        CameraPose {
            position: pos.to_array(),
            forward: forward.to_array(),
            up: [0.0, 1.0, 0.0],
            fov_y_radians: FOV_Y_RADIANS,
        }
    }

    /// Advance the simulation by `dt` seconds (clamped to `0..=0.25` by the caller) at the given
    /// fly speed, then update the pose. The first call arrives with `dt == 0`.
    pub(crate) fn advance(&mut self, dt: f32, settings: &Settings, path: &FlightPath) {
        if !self.started {
            self.started = true;
            self.yaw_rate = 0.0;
            self.bank = 0.0;
        }
        if dt <= 0.0 {
            // First frame or a stall: reset the banking state; the pose stands still (bank 0) and
            // `prev == current` so the motion-blur reprojection sees zero velocity.
            self.yaw_rate = 0.0;
            self.bank = 0.0;
            let pose = self.pose_at(path, settings.fly_speed);
            self.pose = pose;
            self.prev_pose = pose;
            return;
        }

        let span = path.len_segments() as f32;
        self.s = (self.s + settings.fly_speed * dt).rem_euclid(span);

        let pos = Vec3::from_array(path.position(self.s));
        let look = Vec3::from_array(path.position(self.s + settings.fly_speed * LOOKAHEAD_SECONDS));
        let fallback_fwd = Vec3::from_array(self.pose.forward);
        let forward = safe_norm(look - pos, fallback_fwd);

        // Yaw estimate from tangent directions `TANGENT_EPS` apart (legacy updateAtTime).
        let tp = Vec3::from_array(path.position(self.s - TANGENT_EPS))
            - Vec3::from_array(path.position(self.s - 2.0 * TANGENT_EPS));
        let tn = Vec3::from_array(path.position(self.s + TANGENT_EPS))
            - Vec3::from_array(path.position(self.s));
        let a = safe_norm(tp, forward);
        let b = safe_norm(tn, forward);
        let mut signed_yaw = a.cross(b).dot(Vec3::Y);
        if signed_yaw.abs() > YAW_ARTIFACT_THRESHOLD {
            signed_yaw = 0.0;
        }
        let raw_rate = signed_yaw / TANGENT_EPS;

        // Yaw-rate low-pass, tau = 0.28 s.
        self.yaw_rate += (raw_rate - self.yaw_rate) * (1.0 - (-dt / YAW_TAU).exp());

        // Target bank = -yawRate * strength, clamped to +-bank_max_degrees.
        let max_bank = settings.bank_max_degrees.to_radians();
        let target = (-self.yaw_rate * settings.bank_strength).clamp(-max_bank, max_bank);

        // Bank low-pass, tau = max(0.12, bank_smoothing).
        let tau = settings.bank_smoothing.max(0.12);
        self.bank += (target - self.bank) * (1.0 - (-dt / tau).exp());

        let up = if self.bank.abs() > 1e-4 {
            let rolled = rotate_about(Vec3::Y, forward, self.bank);
            if rolled.dot(Vec3::Y) < 0.15 {
                Vec3::Y
            } else {
                rolled.normalize()
            }
        } else {
            Vec3::Y
        };

        self.prev_pose = self.pose;
        self.pose = CameraPose {
            position: pos.to_array(),
            forward: forward.to_array(),
            up: up.to_array(),
            fov_y_radians: FOV_Y_RADIANS,
        };
    }

    /// Current camera pose.
    pub(crate) fn camera(&self) -> CameraPose {
        self.pose
    }

    /// Previous frame's camera pose (motion-blur reprojection).
    pub(crate) fn prev_camera(&self) -> CameraPose {
        self.prev_pose
    }
}

/// `v.normalize()` unless `v` is degenerate, in which case `fallback` (already unit) is returned.
fn safe_norm(v: Vec3, fallback: Vec3) -> Vec3 {
    if v.length_squared() < 1e-10 {
        fallback
    } else {
        v.normalize()
    }
}

/// Rodrigues rotation of `v` about the unit `axis` by `angle`.
fn rotate_about(v: Vec3, axis: Vec3, angle: f32) -> Vec3 {
    let c = angle.cos();
    let s = angle.sin();
    v * c + axis.cross(v) * s + axis * axis.dot(v) * (1.0 - c)
}
