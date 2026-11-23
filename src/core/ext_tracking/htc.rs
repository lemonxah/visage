use super::unified::{UnifiedExpressions, UnifiedShapeAccessors, UnifiedShapes, NUM_SHAPES};

mod xr {
    pub mod sys {
        pub const FACIAL_EXPRESSION_EYE_COUNT_HTC: usize = 14;
        pub const FACIAL_EXPRESSION_LIP_COUNT_HTC: usize = 37;
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub struct EyeExpressionHTC(i32);

    impl EyeExpressionHTC {
        pub const LEFT_BLINK: Self = Self(0);
        pub const LEFT_WIDE: Self = Self(1);
        pub const RIGHT_BLINK: Self = Self(2);
        pub const RIGHT_WIDE: Self = Self(3);
        pub const LEFT_SQUEEZE: Self = Self(4);
        pub const RIGHT_SQUEEZE: Self = Self(5);
        pub const LEFT_DOWN: Self = Self(6);
        pub const RIGHT_DOWN: Self = Self(7);
        pub const LEFT_OUT: Self = Self(8);
        pub const RIGHT_IN: Self = Self(9);
        pub const LEFT_IN: Self = Self(10);
        pub const RIGHT_OUT: Self = Self(11);
        pub const LEFT_UP: Self = Self(12);
        pub const RIGHT_UP: Self = Self(13);

        pub const fn into_raw(self) -> i32 {
            self.0
        }
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub struct LipExpressionHTC(i32);

    impl LipExpressionHTC {
        pub const JAW_RIGHT: Self = Self(0);
        pub const JAW_LEFT: Self = Self(1);
        pub const JAW_FORWARD: Self = Self(2);
        pub const JAW_OPEN: Self = Self(3);
        pub const MOUTH_APE_SHAPE: Self = Self(4);
        pub const MOUTH_UPPER_RIGHT: Self = Self(5);
        pub const MOUTH_UPPER_LEFT: Self = Self(6);
        pub const MOUTH_LOWER_RIGHT: Self = Self(7);
        pub const MOUTH_LOWER_LEFT: Self = Self(8);
        pub const MOUTH_UPPER_OVERTURN: Self = Self(9);
        pub const MOUTH_LOWER_OVERTURN: Self = Self(10);
        pub const MOUTH_POUT: Self = Self(11);
        pub const MOUTH_SMILE_RIGHT: Self = Self(12);
        pub const MOUTH_SMILE_LEFT: Self = Self(13);
        pub const MOUTH_SAD_RIGHT: Self = Self(14);
        pub const MOUTH_SAD_LEFT: Self = Self(15);
        pub const CHEEK_PUFF_RIGHT: Self = Self(16);
        pub const CHEEK_PUFF_LEFT: Self = Self(17);
        pub const CHEEK_SUCK: Self = Self(18);
        pub const MOUTH_UPPER_UPRIGHT: Self = Self(19);
        pub const MOUTH_UPPER_UPLEFT: Self = Self(20);
        pub const MOUTH_LOWER_DOWNRIGHT: Self = Self(21);
        pub const MOUTH_LOWER_DOWNLEFT: Self = Self(22);
        pub const MOUTH_UPPER_INSIDE: Self = Self(23);
        pub const MOUTH_LOWER_INSIDE: Self = Self(24);
        pub const MOUTH_LOWER_OVERLAY: Self = Self(25);
        pub const TONGUE_LONGSTEP1: Self = Self(26);
        pub const TONGUE_LEFT: Self = Self(27);
        pub const TONGUE_RIGHT: Self = Self(28);
        pub const TONGUE_UP: Self = Self(29);
        pub const TONGUE_DOWN: Self = Self(30);
        pub const TONGUE_ROLL: Self = Self(31);
        pub const TONGUE_LONGSTEP2: Self = Self(32);
        pub const TONGUE_UPRIGHT_MORPH: Self = Self(33);
        pub const TONGUE_UPLEFT_MORPH: Self = Self(34);
        pub const TONGUE_DOWNRIGHT_MORPH: Self = Self(35);
        pub const TONGUE_DOWNLEFT_MORPH: Self = Self(36);

        pub const fn into_raw(self) -> i32 {
            self.0
        }
    }
}

pub(crate) struct HtcFacialData {
    pub(super) eye: Option<[f32; xr::sys::FACIAL_EXPRESSION_EYE_COUNT_HTC]>,
    pub(super) lip: Option<[f32; xr::sys::FACIAL_EXPRESSION_LIP_COUNT_HTC]>,
}

impl HtcFacialData {
    pub(crate) fn from_slices(eye: Option<&[f32]>, lip: Option<&[f32]>) -> Option<Self> {
        fn take<const N: usize>(values: Option<&[f32]>) -> Option<[f32; N]> {
            let mut arr: [f32; N] = values?.get(..N)?.try_into().ok()?;
            arr.iter_mut().for_each(|v| *v = super::arkit::sanitize(*v));
            Some(arr)
        }
        let eye = take(eye);
        let lip = take(lip);
        (eye.is_some() || lip.is_some()).then_some(Self { eye, lip })
    }

    #[inline(always)]
    fn eyef(&self, idx: xr::EyeExpressionHTC) -> f32 {
        self.eye
            .map(|arr| arr[idx.into_raw() as usize])
            .unwrap_or_default()
    }

    #[inline(always)]
    fn lipf(&self, idx: xr::LipExpressionHTC) -> f32 {
        self.lip
            .map(|arr| arr[idx.into_raw() as usize])
            .unwrap_or_default()
    }
}

pub(crate) fn htc_to_unified(d: &HtcFacialData) -> UnifiedShapes {
    let mut shapes: UnifiedShapes = [0.0; NUM_SHAPES];

    shapes.setu(
        UnifiedExpressions::EyeRightX,
        d.eyef(xr::EyeExpressionHTC::RIGHT_OUT) - d.eyef(xr::EyeExpressionHTC::RIGHT_IN),
    );
    shapes.setu(
        UnifiedExpressions::EyeLeftX,
        d.eyef(xr::EyeExpressionHTC::LEFT_IN) - d.eyef(xr::EyeExpressionHTC::LEFT_OUT),
    );
    shapes.setu(
        UnifiedExpressions::EyeY,
        (d.eyef(xr::EyeExpressionHTC::LEFT_UP) + d.eyef(xr::EyeExpressionHTC::RIGHT_UP)
            - d.eyef(xr::EyeExpressionHTC::LEFT_DOWN)
            - d.eyef(xr::EyeExpressionHTC::RIGHT_DOWN))
            / 2.0,
    );

    shapes.setu(
        UnifiedExpressions::EyeClosedLeft,
        d.eyef(xr::EyeExpressionHTC::LEFT_BLINK),
    );
    shapes.setu(
        UnifiedExpressions::EyeClosedRight,
        d.eyef(xr::EyeExpressionHTC::RIGHT_BLINK),
    );

    shapes.setu(
        UnifiedExpressions::EyeSquintRight,
        d.eyef(xr::EyeExpressionHTC::RIGHT_SQUEEZE),
    );
    shapes.setu(
        UnifiedExpressions::EyeSquintLeft,
        d.eyef(xr::EyeExpressionHTC::LEFT_SQUEEZE),
    );
    shapes.setu(
        UnifiedExpressions::EyeWideRight,
        d.eyef(xr::EyeExpressionHTC::RIGHT_WIDE),
    );
    shapes.setu(
        UnifiedExpressions::EyeWideLeft,
        d.eyef(xr::EyeExpressionHTC::LEFT_WIDE),
    );

    shapes.setu(
        UnifiedExpressions::BrowPinchRight,
        d.eyef(xr::EyeExpressionHTC::RIGHT_SQUEEZE),
    );
    shapes.setu(
        UnifiedExpressions::BrowPinchLeft,
        d.eyef(xr::EyeExpressionHTC::LEFT_SQUEEZE),
    );
    shapes.setu(
        UnifiedExpressions::BrowLowererRight,
        d.eyef(xr::EyeExpressionHTC::RIGHT_BLINK),
    );
    shapes.setu(
        UnifiedExpressions::BrowLowererLeft,
        d.eyef(xr::EyeExpressionHTC::LEFT_BLINK),
    );

    shapes.setu(
        UnifiedExpressions::CheekPuffRight,
        d.lipf(xr::LipExpressionHTC::CHEEK_PUFF_RIGHT),
    );
    shapes.setu(
        UnifiedExpressions::CheekPuffLeft,
        d.lipf(xr::LipExpressionHTC::CHEEK_PUFF_LEFT),
    );
    shapes.setu(
        UnifiedExpressions::CheekSuckRight,
        d.lipf(xr::LipExpressionHTC::CHEEK_SUCK),
    );
    shapes.setu(
        UnifiedExpressions::CheekSuckLeft,
        d.lipf(xr::LipExpressionHTC::CHEEK_SUCK),
    );

    shapes.setu(
        UnifiedExpressions::JawOpen,
        d.lipf(xr::LipExpressionHTC::JAW_OPEN),
    );
    shapes.setu(
        UnifiedExpressions::JawRight,
        d.lipf(xr::LipExpressionHTC::JAW_RIGHT),
    );
    shapes.setu(
        UnifiedExpressions::JawLeft,
        d.lipf(xr::LipExpressionHTC::JAW_LEFT),
    );
    shapes.setu(
        UnifiedExpressions::JawForward,
        d.lipf(xr::LipExpressionHTC::JAW_FORWARD),
    );
    shapes.setu(
        UnifiedExpressions::MouthClosed,
        d.lipf(xr::LipExpressionHTC::MOUTH_APE_SHAPE),
    );

    let suck_upper = d.lipf(xr::LipExpressionHTC::MOUTH_UPPER_INSIDE);
    shapes.setu(UnifiedExpressions::LipSuckUpperRight, suck_upper);
    shapes.setu(UnifiedExpressions::LipSuckUpperLeft, suck_upper);

    let suck_lower = d.lipf(xr::LipExpressionHTC::MOUTH_LOWER_INSIDE);
    shapes.setu(UnifiedExpressions::LipSuckLowerRight, suck_lower);
    shapes.setu(UnifiedExpressions::LipSuckLowerLeft, suck_lower);

    let upper_funnel = d.lipf(xr::LipExpressionHTC::MOUTH_LOWER_OVERTURN);
    shapes.setu(UnifiedExpressions::LipFunnelUpperRight, upper_funnel);
    shapes.setu(UnifiedExpressions::LipFunnelUpperLeft, upper_funnel);

    let lower_funnel = d.lipf(xr::LipExpressionHTC::MOUTH_LOWER_OVERTURN);
    shapes.setu(UnifiedExpressions::LipFunnelLowerRight, lower_funnel);
    shapes.setu(UnifiedExpressions::LipFunnelLowerLeft, lower_funnel);

    let pout = d.lipf(xr::LipExpressionHTC::MOUTH_POUT);
    shapes.setu(UnifiedExpressions::LipPuckerUpperRight, pout);
    shapes.setu(UnifiedExpressions::LipPuckerUpperLeft, pout);
    shapes.setu(UnifiedExpressions::LipPuckerLowerRight, pout);
    shapes.setu(UnifiedExpressions::LipPuckerLowerLeft, pout);

    shapes.setu(
        UnifiedExpressions::MouthLowerDownRight,
        d.lipf(xr::LipExpressionHTC::MOUTH_LOWER_DOWNRIGHT),
    );
    shapes.setu(
        UnifiedExpressions::MouthLowerDownLeft,
        d.lipf(xr::LipExpressionHTC::MOUTH_LOWER_DOWNLEFT),
    );

    let mouth_upper_up_right = d.lipf(xr::LipExpressionHTC::MOUTH_UPPER_UPRIGHT);
    let mouth_upper_up_left = d.lipf(xr::LipExpressionHTC::MOUTH_UPPER_UPLEFT);

    shapes.setu(UnifiedExpressions::MouthUpperUpRight, mouth_upper_up_right);
    shapes.setu(UnifiedExpressions::MouthUpperUpLeft, mouth_upper_up_left);
    shapes.setu(
        UnifiedExpressions::MouthUpperDeepenRight,
        mouth_upper_up_right,
    );
    shapes.setu(
        UnifiedExpressions::MouthUpperDeepenLeft,
        mouth_upper_up_left,
    );

    shapes.setu(
        UnifiedExpressions::MouthUpperRight,
        d.lipf(xr::LipExpressionHTC::MOUTH_UPPER_RIGHT),
    );
    shapes.setu(
        UnifiedExpressions::MouthUpperLeft,
        d.lipf(xr::LipExpressionHTC::MOUTH_UPPER_LEFT),
    );
    shapes.setu(
        UnifiedExpressions::MouthLowerRight,
        d.lipf(xr::LipExpressionHTC::MOUTH_LOWER_RIGHT),
    );
    shapes.setu(
        UnifiedExpressions::MouthLowerLeft,
        d.lipf(xr::LipExpressionHTC::MOUTH_LOWER_LEFT),
    );

    let smile_left = d.lipf(xr::LipExpressionHTC::MOUTH_SMILE_RIGHT);
    shapes.setu(UnifiedExpressions::MouthCornerPullLeft, smile_left);
    shapes.setu(UnifiedExpressions::MouthCornerSlantLeft, smile_left);

    let smile_right = d.lipf(xr::LipExpressionHTC::MOUTH_SMILE_RIGHT);
    shapes.setu(UnifiedExpressions::MouthCornerPullRight, smile_right);
    shapes.setu(UnifiedExpressions::MouthCornerSlantRight, smile_right);

    let sad_left = d.lipf(xr::LipExpressionHTC::MOUTH_SAD_LEFT);
    shapes.setu(UnifiedExpressions::MouthFrownLeft, sad_left);
    shapes.setu(UnifiedExpressions::MouthStretchLeft, sad_left);

    let sad_right = d.lipf(xr::LipExpressionHTC::MOUTH_SAD_RIGHT);
    shapes.setu(UnifiedExpressions::MouthFrownRight, sad_right);
    shapes.setu(UnifiedExpressions::MouthStretchRight, sad_right);

    let press = (d.lipf(xr::LipExpressionHTC::MOUTH_UPPER_INSIDE)
        + d.lipf(xr::LipExpressionHTC::MOUTH_LOWER_INSIDE))
        / 2.0;
    shapes.setu(UnifiedExpressions::MouthPressRight, press);
    shapes.setu(UnifiedExpressions::MouthPressLeft, press);

    shapes.setu(
        UnifiedExpressions::TongueOut,
        (d.lipf(xr::LipExpressionHTC::TONGUE_LONGSTEP1)
            + d.lipf(xr::LipExpressionHTC::TONGUE_LONGSTEP2))
            / 2.0,
    );
    shapes.setu(
        UnifiedExpressions::TongueUp,
        d.lipf(xr::LipExpressionHTC::TONGUE_UP),
    );
    shapes.setu(
        UnifiedExpressions::TongueDown,
        d.lipf(xr::LipExpressionHTC::TONGUE_DOWN),
    );
    shapes.setu(
        UnifiedExpressions::TongueLeft,
        d.lipf(xr::LipExpressionHTC::TONGUE_LEFT),
    );
    shapes.setu(
        UnifiedExpressions::TongueRight,
        d.lipf(xr::LipExpressionHTC::TONGUE_RIGHT),
    );
    shapes.setu(
        UnifiedExpressions::TongueRoll,
        d.lipf(xr::LipExpressionHTC::TONGUE_ROLL),
    );
    shapes.setu(
        UnifiedExpressions::TongueTwistLeft,
        d.lipf(xr::LipExpressionHTC::TONGUE_UPLEFT_MORPH)
            + d.lipf(xr::LipExpressionHTC::TONGUE_DOWNRIGHT_MORPH),
    );
    shapes.setu(
        UnifiedExpressions::TongueTwistRight,
        d.lipf(xr::LipExpressionHTC::TONGUE_UPRIGHT_MORPH)
            + d.lipf(xr::LipExpressionHTC::TONGUE_DOWNLEFT_MORPH),
    );

    /* does not map:
        UnifiedExpressions::BrowInnerUpLeft,
        UnifiedExpressions::BrowInnerUpRight,
        UnifiedExpressions::BrowOuterUpLeft,
        UnifiedExpressions::BrowOuterUpRight,
        UnifiedExpressions::CheekSquintLeft,
        UnifiedExpressions::CheekSquintRight,
        UnifiedExpressions::NoseSneerLeft,
        UnifiedExpressions::NoseSneerRight,
        UnifiedExpressions::MouthDimpleLeft,
        UnifiedExpressions::MouthDimpleRight,
        UnifiedExpressions::MouthTightenerLeft,
        UnifiedExpressions::MouthTightenerRight,
    */

    shapes
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(feature = "openxr")]
    #[test]
    fn local_htc_indices_match_openxr() {
        use openxr as oxr;
        assert_eq!(
            xr::sys::FACIAL_EXPRESSION_EYE_COUNT_HTC,
            oxr::sys::FACIAL_EXPRESSION_EYE_COUNT_HTC
        );
        assert_eq!(
            xr::sys::FACIAL_EXPRESSION_LIP_COUNT_HTC,
            oxr::sys::FACIAL_EXPRESSION_LIP_COUNT_HTC
        );
        assert_eq!(
            xr::EyeExpressionHTC::RIGHT_UP.into_raw(),
            oxr::EyeExpressionHTC::RIGHT_UP.into_raw()
        );
        assert_eq!(
            xr::EyeExpressionHTC::LEFT_IN.into_raw(),
            oxr::EyeExpressionHTC::LEFT_IN.into_raw()
        );
        assert_eq!(
            xr::LipExpressionHTC::MOUTH_LOWER_INSIDE.into_raw(),
            oxr::LipExpressionHTC::MOUTH_LOWER_INSIDE.into_raw()
        );
        assert_eq!(
            xr::LipExpressionHTC::TONGUE_DOWNLEFT_MORPH.into_raw(),
            oxr::LipExpressionHTC::TONGUE_DOWNLEFT_MORPH.into_raw()
        );
    }

    #[test]
    fn from_slices_requires_full_arrays_and_sanitizes() {
        assert!(HtcFacialData::from_slices(None, None).is_none());
        assert!(HtcFacialData::from_slices(Some(&[0.0; 13]), Some(&[0.0; 36])).is_none());
        let mut lip = vec![0.0; 37];
        lip[xr::LipExpressionHTC::JAW_OPEN.into_raw() as usize] = 2.0;
        let data = HtcFacialData::from_slices(Some(&[0.0; 10]), Some(&lip)).unwrap();
        assert!(data.eye.is_none());
        let shapes = htc_to_unified(&data);
        assert_eq!(shapes.getu(UnifiedExpressions::JawOpen), 1.0);
        let mut eye = vec![0.0; 14];
        eye[0] = f32::NAN;
        eye[2] = 0.5;
        let data = HtcFacialData::from_slices(Some(&eye), None).unwrap();
        let shapes = htc_to_unified(&data);
        assert_eq!(shapes.getu(UnifiedExpressions::EyeClosedLeft), 0.0);
        assert_eq!(shapes.getu(UnifiedExpressions::EyeClosedRight), 0.5);
    }
}
