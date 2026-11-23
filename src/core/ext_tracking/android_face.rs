use std::{
    ffi::{c_char, c_void, CStr},
    ptr,
    sync::atomic::{AtomicBool, Ordering},
};

use openxr as xr;
use xr::sys;

use super::{
    face2_fb::face2_fb_to_unified,
    unified::{UnifiedExpressions, UnifiedShapeAccessors, UnifiedShapes},
};

pub(crate) const EXTENSION_NAME: &str = "XR_ANDROID_face_tracking";
pub(crate) const PARAMETER_COUNT: usize = 68;
const FB_COMPATIBLE_COUNT: usize = 63;
const REGION_CONFIDENCE_COUNT: usize = 3;

const TYPE_FACE_TRACKER_CREATE_INFO: i32 = 1000458000;
const TYPE_FACE_STATE_GET_INFO: i32 = 1000458001;
const TYPE_FACE_STATE: i32 = 1000458002;
const TYPE_SYSTEM_FACE_TRACKING_PROPERTIES: i32 = 1000458003;

pub(crate) const PARAMETER_NAMES: [&str; PARAMETER_COUNT] = [
    "BROW_LOWERER_L",
    "BROW_LOWERER_R",
    "CHEEK_PUFF_L",
    "CHEEK_PUFF_R",
    "CHEEK_RAISER_L",
    "CHEEK_RAISER_R",
    "CHEEK_SUCK_L",
    "CHEEK_SUCK_R",
    "CHIN_RAISER_B",
    "CHIN_RAISER_T",
    "DIMPLER_L",
    "DIMPLER_R",
    "EYES_CLOSED_L",
    "EYES_CLOSED_R",
    "EYES_LOOK_DOWN_L",
    "EYES_LOOK_DOWN_R",
    "EYES_LOOK_LEFT_L",
    "EYES_LOOK_LEFT_R",
    "EYES_LOOK_RIGHT_L",
    "EYES_LOOK_RIGHT_R",
    "EYES_LOOK_UP_L",
    "EYES_LOOK_UP_R",
    "INNER_BROW_RAISER_L",
    "INNER_BROW_RAISER_R",
    "JAW_DROP",
    "JAW_SIDEWAYS_LEFT",
    "JAW_SIDEWAYS_RIGHT",
    "JAW_THRUST",
    "LID_TIGHTENER_L",
    "LID_TIGHTENER_R",
    "LIP_CORNER_DEPRESSOR_L",
    "LIP_CORNER_DEPRESSOR_R",
    "LIP_CORNER_PULLER_L",
    "LIP_CORNER_PULLER_R",
    "LIP_FUNNELER_LB",
    "LIP_FUNNELER_LT",
    "LIP_FUNNELER_RB",
    "LIP_FUNNELER_RT",
    "LIP_PRESSOR_L",
    "LIP_PRESSOR_R",
    "LIP_PUCKER_L",
    "LIP_PUCKER_R",
    "LIP_STRETCHER_L",
    "LIP_STRETCHER_R",
    "LIP_SUCK_LB",
    "LIP_SUCK_LT",
    "LIP_SUCK_RB",
    "LIP_SUCK_RT",
    "LIP_TIGHTENER_L",
    "LIP_TIGHTENER_R",
    "LIPS_TOWARD",
    "LOWER_LIP_DEPRESSOR_L",
    "LOWER_LIP_DEPRESSOR_R",
    "MOUTH_LEFT",
    "MOUTH_RIGHT",
    "NOSE_WRINKLER_L",
    "NOSE_WRINKLER_R",
    "OUTER_BROW_RAISER_L",
    "OUTER_BROW_RAISER_R",
    "UPPER_LID_RAISER_L",
    "UPPER_LID_RAISER_R",
    "UPPER_LIP_RAISER_L",
    "UPPER_LIP_RAISER_R",
    "TONGUE_OUT",
    "TONGUE_LEFT",
    "TONGUE_RIGHT",
    "TONGUE_UP",
    "TONGUE_DOWN",
];

const TONGUE: [(usize, UnifiedExpressions); 5] = [
    (63, UnifiedExpressions::TongueOut),
    (64, UnifiedExpressions::TongueLeft),
    (65, UnifiedExpressions::TongueRight),
    (66, UnifiedExpressions::TongueUp),
    (67, UnifiedExpressions::TongueDown),
];

pub(crate) fn android_to_unified(parameters: &[f32]) -> Option<UnifiedShapes> {
    if parameters.len() < PARAMETER_COUNT {
        return None;
    }
    let mut clean = [0f32; PARAMETER_COUNT];
    for (dst, src) in clean.iter_mut().zip(parameters) {
        *dst = if src.is_finite() {
            src.clamp(0.0, 1.0)
        } else {
            0.0
        };
    }
    let mut shapes = face2_fb_to_unified(&clean[..FB_COMPATIBLE_COUNT])?;
    shapes.setu(UnifiedExpressions::TongueCurlUp, 0.0);
    for (index, expression) in TONGUE {
        shapes.setu(expression, clean[index]);
    }
    Some(shapes)
}

#[repr(transparent)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct FaceTrackerHandle(u64);

#[repr(C)]
struct FaceTrackerCreateInfo {
    ty: i32,
    next: *const c_void,
}

#[repr(C)]
struct FaceStateGetInfo {
    ty: i32,
    next: *const c_void,
    time: sys::Time,
}

#[repr(C)]
struct FaceState {
    ty: i32,
    next: *mut c_void,
    parameters_capacity_input: u32,
    parameters_count_output: u32,
    parameters: *mut f32,
    face_tracking_state: i32,
    sample_time: sys::Time,
    is_valid: sys::Bool32,
    region_confidences_capacity_input: u32,
    region_confidences_count_output: u32,
    region_confidences: *mut f32,
}

#[repr(C)]
struct SystemFaceTrackingProperties {
    ty: i32,
    next: *mut c_void,
    supports_face_tracking: sys::Bool32,
}

type CreateFaceTracker = unsafe extern "system" fn(
    sys::Session,
    *const FaceTrackerCreateInfo,
    *mut FaceTrackerHandle,
) -> sys::Result;
type DestroyFaceTracker = unsafe extern "system" fn(FaceTrackerHandle) -> sys::Result;
type GetFaceState = unsafe extern "system" fn(
    FaceTrackerHandle,
    *const FaceStateGetInfo,
    *mut FaceState,
) -> sys::Result;

pub(crate) fn is_available(extensions: &xr::ExtensionSet) -> bool {
    extensions.other.iter().any(|name| name == EXTENSION_NAME)
}

unsafe fn load<T: Copy>(instance: &xr::Instance, name: &CStr) -> anyhow::Result<T> {
    debug_assert_eq!(
        std::mem::size_of::<T>(),
        std::mem::size_of::<sys::pfn::VoidFunction>()
    );
    let mut function: Option<sys::pfn::VoidFunction> = None;
    let res = (instance.entry().fp().get_instance_proc_addr)(
        instance.as_raw(),
        name.as_ptr() as *const c_char,
        &mut function,
    );
    match function {
        Some(function) if res == sys::Result::SUCCESS => Ok(std::mem::transmute_copy::<
            sys::pfn::VoidFunction,
            T,
        >(&function)),
        _ => anyhow::bail!("{} unavailable: {:?}", name.to_string_lossy(), res),
    }
}

pub(crate) struct AndroidFaceTracker {
    destroy: DestroyFaceTracker,
    get_state: GetFaceState,
    handle: FaceTrackerHandle,
    warned: AtomicBool,
}

impl AndroidFaceTracker {
    pub fn new(
        instance: &xr::Instance,
        system: xr::SystemId,
        session: sys::Session,
    ) -> anyhow::Result<Self> {
        let enabled = instance
            .entry()
            .enumerate_extensions()
            .map(|available| is_available(&available))
            .unwrap_or(false);
        if !enabled {
            anyhow::bail!("Extension not supported.");
        }

        let mut props = SystemFaceTrackingProperties {
            ty: TYPE_SYSTEM_FACE_TRACKING_PROPERTIES,
            next: ptr::null_mut(),
            supports_face_tracking: sys::Bool32::from_raw(0),
        };
        let res = unsafe {
            let mut system_props = sys::SystemProperties {
                ty: sys::SystemProperties::TYPE,
                next: &mut props as *mut SystemFaceTrackingProperties as *mut c_void,
                ..std::mem::zeroed()
            };
            (instance.fp().get_system_properties)(instance.as_raw(), system, &mut system_props)
        };
        if res != sys::Result::SUCCESS {
            anyhow::bail!("Could not query system properties: {:?}", res);
        }
        if props.supports_face_tracking.into_raw() == 0 {
            anyhow::bail!("Face tracking not supported by this system.");
        }

        let (create, destroy, get_state) = unsafe {
            (
                load::<CreateFaceTracker>(instance, c"xrCreateFaceTrackerANDROID")?,
                load::<DestroyFaceTracker>(instance, c"xrDestroyFaceTrackerANDROID")?,
                load::<GetFaceState>(instance, c"xrGetFaceStateANDROID")?,
            )
        };

        let info = FaceTrackerCreateInfo {
            ty: TYPE_FACE_TRACKER_CREATE_INFO,
            next: ptr::null(),
        };
        let mut handle = FaceTrackerHandle(0);
        let res = unsafe { create(session, &info, &mut handle) };
        if res != sys::Result::SUCCESS || handle.0 == 0 {
            anyhow::bail!("Could not initialize: {:?}", res);
        }

        log::info!("Using ANDROID_face_tracking for face.");

        Ok(Self {
            destroy,
            get_state,
            handle,
            warned: AtomicBool::new(false),
        })
    }

    pub fn get_parameters(&self, time: xr::Time) -> Option<[f32; PARAMETER_COUNT]> {
        let mut parameters = [0f32; PARAMETER_COUNT];
        let mut confidences = [0f32; REGION_CONFIDENCE_COUNT];
        let mut state = FaceState {
            ty: TYPE_FACE_STATE,
            next: ptr::null_mut(),
            parameters_capacity_input: PARAMETER_COUNT as u32,
            parameters_count_output: 0,
            parameters: parameters.as_mut_ptr(),
            face_tracking_state: 0,
            sample_time: xr::Time::from_nanos(0),
            is_valid: sys::Bool32::from_raw(0),
            region_confidences_capacity_input: REGION_CONFIDENCE_COUNT as u32,
            region_confidences_count_output: 0,
            region_confidences: confidences.as_mut_ptr(),
        };
        let info = FaceStateGetInfo {
            ty: TYPE_FACE_STATE_GET_INFO,
            next: ptr::null(),
            time,
        };
        let res = unsafe { (self.get_state)(self.handle, &info, &mut state) };
        if res != sys::Result::SUCCESS {
            if !self.warned.swap(true, Ordering::Relaxed) {
                log::warn!(
                    "ANDROID_face_tracking: xrGetFaceStateANDROID failed: {:?}",
                    res
                );
            } else {
                log::debug!(
                    "ANDROID_face_tracking: xrGetFaceStateANDROID failed: {:?}",
                    res
                );
            }
            return None;
        }
        if state.is_valid.into_raw() == 0
            || (state.parameters_count_output as usize) < FB_COMPATIBLE_COUNT
        {
            return None;
        }
        Some(parameters)
    }
}

impl Drop for AndroidFaceTracker {
    fn drop(&mut self) {
        unsafe {
            (self.destroy)(self.handle);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn struct_layouts_match_the_openxr_abi() {
        assert_eq!(std::mem::size_of::<FaceTrackerCreateInfo>(), 16);
        assert_eq!(std::mem::size_of::<FaceStateGetInfo>(), 24);
        assert_eq!(std::mem::size_of::<SystemFaceTrackingProperties>(), 24);
        assert_eq!(std::mem::size_of::<FaceState>(), 72);
        assert_eq!(std::mem::offset_of!(FaceState, parameters), 24);
        assert_eq!(std::mem::offset_of!(FaceState, sample_time), 40);
        assert_eq!(std::mem::offset_of!(FaceState, region_confidences), 64);
    }

    #[test]
    fn mapping_table_indices_are_in_range_and_unique() {
        let names: HashSet<_> = PARAMETER_NAMES.iter().collect();
        assert_eq!(names.len(), PARAMETER_COUNT);
        let indices: HashSet<_> = TONGUE.iter().map(|(i, _)| *i).collect();
        let targets: HashSet<_> = TONGUE.iter().map(|(_, e)| *e as usize).collect();
        assert_eq!(indices.len(), TONGUE.len());
        assert_eq!(targets.len(), TONGUE.len());
        for (index, _) in TONGUE {
            assert!((FB_COMPATIBLE_COUNT..PARAMETER_COUNT).contains(&index));
            assert!(PARAMETER_NAMES[index].starts_with("TONGUE_"));
        }
        assert!(PARAMETER_NAMES[..FB_COMPATIBLE_COUNT]
            .iter()
            .all(|name| !name.starts_with("TONGUE_")));
    }

    #[test]
    fn fb_compatible_prefix_matches_fb_names() {
        let fb = &super::super::steamlink::FB_NAMES;
        for (index, android) in PARAMETER_NAMES[..FB_COMPATIBLE_COUNT].iter().enumerate() {
            let squashed: String = android
                .split('_')
                .map(|part| part.to_ascii_lowercase())
                .collect();
            assert_eq!(
                squashed,
                fb[index].to_ascii_lowercase(),
                "index {index}: {android} vs {}",
                fb[index]
            );
        }
    }

    #[test]
    fn android_parameters_map_through_fb_and_tongue() {
        let mut params = [0f32; PARAMETER_COUNT];
        params[24] = 0.7;
        params[12] = 0.5;
        params[63] = 0.9;
        params[66] = 0.3;
        let shapes = android_to_unified(&params).unwrap();
        assert_eq!(shapes.getu(UnifiedExpressions::JawOpen), 0.7);
        assert_eq!(shapes.getu(UnifiedExpressions::EyeClosedLeft), 0.5);
        assert_eq!(shapes.getu(UnifiedExpressions::TongueOut), 0.9);
        assert_eq!(shapes.getu(UnifiedExpressions::TongueUp), 0.3);
        assert_eq!(shapes.getu(UnifiedExpressions::TongueCurlUp), 0.0);
    }

    #[test]
    fn android_mapping_rejects_short_and_sanitizes_values() {
        assert!(android_to_unified(&[0.0; 63]).is_none());
        let mut params = [0f32; PARAMETER_COUNT];
        params[24] = f32::NAN;
        params[63] = 4.0;
        params[64] = -1.0;
        let shapes = android_to_unified(&params).unwrap();
        assert_eq!(shapes.getu(UnifiedExpressions::JawOpen), 0.0);
        assert_eq!(shapes.getu(UnifiedExpressions::TongueOut), 1.0);
        assert_eq!(shapes.getu(UnifiedExpressions::TongueLeft), 0.0);
    }
}
