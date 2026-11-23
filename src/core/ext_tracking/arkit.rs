use std::collections::HashMap;

use once_cell::sync::Lazy;
use strum::{EnumCount, EnumIter, IntoEnumIterator, IntoStaticStr};

use super::unified::{UnifiedExpressions as U, UnifiedShapeAccessors, UnifiedShapes, NUM_SHAPES};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, EnumCount, EnumIter, IntoStaticStr)]
#[repr(usize)]
pub(crate) enum ArkitShape {
    EyeBlinkLeft,
    EyeLookDownLeft,
    EyeLookInLeft,
    EyeLookOutLeft,
    EyeLookUpLeft,
    EyeSquintLeft,
    EyeWideLeft,
    EyeBlinkRight,
    EyeLookDownRight,
    EyeLookInRight,
    EyeLookOutRight,
    EyeLookUpRight,
    EyeSquintRight,
    EyeWideRight,
    JawForward,
    JawRight,
    JawLeft,
    JawOpen,
    MouthClose,
    MouthFunnel,
    MouthPucker,
    MouthLeft,
    MouthRight,
    MouthSmileLeft,
    MouthSmileRight,
    MouthFrownLeft,
    MouthFrownRight,
    MouthDimpleLeft,
    MouthDimpleRight,
    MouthStretchLeft,
    MouthStretchRight,
    MouthRollLower,
    MouthRollUpper,
    MouthShrugLower,
    MouthShrugUpper,
    MouthPressLeft,
    MouthPressRight,
    MouthLowerDownLeft,
    MouthLowerDownRight,
    MouthUpperUpLeft,
    MouthUpperUpRight,
    BrowDownLeft,
    BrowDownRight,
    BrowInnerUp,
    BrowOuterUpLeft,
    BrowOuterUpRight,
    CheekPuff,
    CheekSquintLeft,
    CheekSquintRight,
    NoseSneerLeft,
    NoseSneerRight,
    TongueOut,
}

pub(crate) const ARKIT_COUNT: usize = ArkitShape::COUNT;

pub(crate) type ArkitWeights = [f32; ARKIT_COUNT];

pub(crate) const PICO_ORDER: [ArkitShape; ARKIT_COUNT] = {
    use ArkitShape::*;
    [
        EyeLookDownLeft,
        NoseSneerLeft,
        EyeLookInLeft,
        BrowInnerUp,
        BrowDownRight,
        MouthClose,
        MouthLowerDownRight,
        JawOpen,
        MouthUpperUpRight,
        MouthShrugUpper,
        MouthFunnel,
        EyeLookInRight,
        EyeLookDownRight,
        NoseSneerRight,
        MouthRollUpper,
        JawRight,
        BrowDownLeft,
        MouthShrugLower,
        MouthRollLower,
        MouthSmileLeft,
        MouthPressLeft,
        MouthSmileRight,
        MouthPressRight,
        MouthDimpleRight,
        MouthLeft,
        JawForward,
        EyeSquintLeft,
        MouthFrownLeft,
        EyeBlinkLeft,
        CheekSquintLeft,
        BrowOuterUpLeft,
        EyeLookUpLeft,
        JawLeft,
        MouthStretchLeft,
        MouthPucker,
        EyeLookUpRight,
        BrowOuterUpRight,
        CheekSquintRight,
        EyeBlinkRight,
        MouthUpperUpLeft,
        MouthFrownRight,
        EyeSquintRight,
        MouthStretchRight,
        CheekPuff,
        EyeLookOutLeft,
        EyeLookOutRight,
        EyeWideRight,
        EyeWideLeft,
        MouthRight,
        MouthDimpleLeft,
        MouthLowerDownLeft,
        TongueOut,
    ]
};

fn normalize(name: &str) -> String {
    let lower = name.trim().to_ascii_lowercase();
    let expanded = if let Some(stem) = lower.strip_suffix("_l") {
        format!("{stem}left")
    } else if let Some(stem) = lower.strip_suffix("_r") {
        format!("{stem}right")
    } else {
        lower
    };
    expanded.chars().filter(|c| *c != '_').collect()
}

static BY_NAME: Lazy<HashMap<String, ArkitShape>> = Lazy::new(|| {
    ArkitShape::iter()
        .map(|shape| (normalize(shape.into()), shape))
        .collect()
});

impl ArkitShape {
    pub(crate) fn from_name(name: &str) -> Option<Self> {
        if name.len() > 64 {
            return None;
        }
        BY_NAME.get(&normalize(name)).copied()
    }
}

pub(crate) fn sanitize(value: f32) -> f32 {
    if value.is_finite() {
        value.clamp(0.0, 1.0)
    } else {
        0.0
    }
}

pub(crate) fn pico_to_arkit(values: &[f32]) -> Option<ArkitWeights> {
    if values.len() < ARKIT_COUNT {
        return None;
    }
    let mut weights = [0.0; ARKIT_COUNT];
    for (shape, value) in PICO_ORDER.iter().zip(values) {
        weights[*shape as usize] = *value;
    }
    Some(weights)
}

pub(crate) fn arkit_to_unified(weights: &ArkitWeights) -> UnifiedShapes {
    use ArkitShape as A;

    let g = |shape: A| sanitize(weights[shape as usize]);
    let mut shapes: UnifiedShapes = [0.0; NUM_SHAPES];

    shapes.setu(U::EyeLeftX, g(A::EyeLookInLeft) - g(A::EyeLookOutLeft));
    shapes.setu(U::EyeRightX, g(A::EyeLookOutRight) - g(A::EyeLookInRight));
    shapes.setu(
        U::EyeY,
        (g(A::EyeLookUpLeft) + g(A::EyeLookUpRight)
            - g(A::EyeLookDownLeft)
            - g(A::EyeLookDownRight))
            * 0.5,
    );
    shapes.setu(U::EyeClosedLeft, g(A::EyeBlinkLeft));
    shapes.setu(U::EyeClosedRight, g(A::EyeBlinkRight));
    shapes.setu(U::EyeSquintLeft, g(A::EyeSquintLeft));
    shapes.setu(U::EyeSquintRight, g(A::EyeSquintRight));
    shapes.setu(U::EyeWideLeft, g(A::EyeWideLeft));
    shapes.setu(U::EyeWideRight, g(A::EyeWideRight));

    shapes.setu(U::BrowPinchLeft, g(A::BrowDownLeft));
    shapes.setu(U::BrowPinchRight, g(A::BrowDownRight));
    shapes.setu(U::BrowLowererLeft, g(A::BrowDownLeft));
    shapes.setu(U::BrowLowererRight, g(A::BrowDownRight));
    shapes.setu(U::BrowInnerUpLeft, g(A::BrowInnerUp));
    shapes.setu(U::BrowInnerUpRight, g(A::BrowInnerUp));
    shapes.setu(U::BrowOuterUpLeft, g(A::BrowOuterUpLeft));
    shapes.setu(U::BrowOuterUpRight, g(A::BrowOuterUpRight));

    shapes.setu(U::CheekSquintLeft, g(A::CheekSquintLeft));
    shapes.setu(U::CheekSquintRight, g(A::CheekSquintRight));
    shapes.setu(U::CheekPuffLeft, g(A::CheekPuff));
    shapes.setu(U::CheekPuffRight, g(A::CheekPuff));

    shapes.setu(U::JawOpen, g(A::JawOpen));
    shapes.setu(U::JawLeft, g(A::JawLeft));
    shapes.setu(U::JawRight, g(A::JawRight));
    shapes.setu(U::JawForward, g(A::JawForward));
    shapes.setu(U::MouthClosed, g(A::MouthClose));

    for e in [U::LipSuckUpperLeft, U::LipSuckUpperRight] {
        shapes.setu(e, g(A::MouthRollUpper));
    }
    for e in [U::LipSuckLowerLeft, U::LipSuckLowerRight] {
        shapes.setu(e, g(A::MouthRollLower));
    }
    for e in [
        U::LipFunnelUpperLeft,
        U::LipFunnelUpperRight,
        U::LipFunnelLowerLeft,
        U::LipFunnelLowerRight,
    ] {
        shapes.setu(e, g(A::MouthFunnel));
    }
    for e in [
        U::LipPuckerUpperLeft,
        U::LipPuckerUpperRight,
        U::LipPuckerLowerLeft,
        U::LipPuckerLowerRight,
    ] {
        shapes.setu(e, g(A::MouthPucker));
    }

    let upper_up_left = (g(A::MouthUpperUpLeft) - g(A::NoseSneerLeft)).max(0.0);
    let upper_up_right = (g(A::MouthUpperUpRight) - g(A::NoseSneerRight)).max(0.0);
    shapes.setu(U::MouthUpperUpLeft, upper_up_left);
    shapes.setu(U::MouthUpperUpRight, upper_up_right);
    shapes.setu(U::MouthUpperDeepenLeft, upper_up_left);
    shapes.setu(U::MouthUpperDeepenRight, upper_up_right);
    shapes.setu(U::NoseSneerLeft, g(A::NoseSneerLeft));
    shapes.setu(U::NoseSneerRight, g(A::NoseSneerRight));

    shapes.setu(U::MouthLowerDownLeft, g(A::MouthLowerDownLeft));
    shapes.setu(U::MouthLowerDownRight, g(A::MouthLowerDownRight));
    shapes.setu(U::MouthUpperLeft, g(A::MouthLeft));
    shapes.setu(U::MouthLowerLeft, g(A::MouthLeft));
    shapes.setu(U::MouthUpperRight, g(A::MouthRight));
    shapes.setu(U::MouthLowerRight, g(A::MouthRight));

    shapes.setu(U::MouthCornerPullLeft, g(A::MouthSmileLeft));
    shapes.setu(U::MouthCornerSlantLeft, g(A::MouthSmileLeft));
    shapes.setu(U::MouthCornerPullRight, g(A::MouthSmileRight));
    shapes.setu(U::MouthCornerSlantRight, g(A::MouthSmileRight));
    shapes.setu(U::MouthFrownLeft, g(A::MouthFrownLeft));
    shapes.setu(U::MouthFrownRight, g(A::MouthFrownRight));
    shapes.setu(U::MouthStretchLeft, g(A::MouthStretchLeft));
    shapes.setu(U::MouthStretchRight, g(A::MouthStretchRight));
    shapes.setu(U::MouthDimpleLeft, g(A::MouthDimpleLeft));
    shapes.setu(U::MouthDimpleRight, g(A::MouthDimpleRight));
    shapes.setu(U::MouthRaiserUpper, g(A::MouthShrugUpper));
    shapes.setu(U::MouthRaiserLower, g(A::MouthShrugLower));
    shapes.setu(U::MouthPressLeft, g(A::MouthPressLeft));
    shapes.setu(U::MouthPressRight, g(A::MouthPressRight));

    shapes.setu(U::TongueOut, g(A::TongueOut));

    shapes
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    fn one_hot(shape: ArkitShape, value: f32) -> ArkitWeights {
        let mut weights = [0.0; ARKIT_COUNT];
        weights[shape as usize] = value;
        weights
    }

    #[test]
    fn canonical_order_matches_apple_list() {
        assert_eq!(ARKIT_COUNT, 52);
        assert_eq!(ArkitShape::EyeBlinkLeft as usize, 0);
        assert_eq!(ArkitShape::EyeBlinkRight as usize, 7);
        assert_eq!(ArkitShape::JawOpen as usize, 17);
        assert_eq!(ArkitShape::BrowDownLeft as usize, 41);
        assert_eq!(ArkitShape::TongueOut as usize, 51);
        for (index, shape) in ArkitShape::iter().enumerate() {
            assert_eq!(shape as usize, index);
        }
    }

    #[test]
    fn pico_order_is_a_permutation_matching_alvr_blink_indices() {
        let unique: HashSet<_> = PICO_ORDER.iter().collect();
        assert_eq!(unique.len(), ARKIT_COUNT);
        assert_eq!(PICO_ORDER[28], ArkitShape::EyeBlinkLeft);
        assert_eq!(PICO_ORDER[38], ArkitShape::EyeBlinkRight);
        assert_eq!(PICO_ORDER[7], ArkitShape::JawOpen);
        assert_eq!(PICO_ORDER[51], ArkitShape::TongueOut);
    }

    #[test]
    fn pico_to_arkit_reorders_and_rejects_short_input() {
        let mut pico = vec![0.0; 52];
        pico[28] = 0.8;
        pico[7] = 0.4;
        let weights = pico_to_arkit(&pico).unwrap();
        assert_eq!(weights[ArkitShape::EyeBlinkLeft as usize], 0.8);
        assert_eq!(weights[ArkitShape::JawOpen as usize], 0.4);
        assert!(pico_to_arkit(&pico[..51]).is_none());
        assert!(pico_to_arkit(&[]).is_none());
    }

    #[test]
    fn names_from_every_protocol_resolve() {
        for shape in ArkitShape::iter() {
            let name: &'static str = shape.into();
            assert_eq!(ArkitShape::from_name(name), Some(shape));
            let mut lower_first = name.to_string();
            lower_first[..1].make_ascii_lowercase();
            assert_eq!(ArkitShape::from_name(&lower_first), Some(shape));
        }
        assert_eq!(
            ArkitShape::from_name("eyeBlink_L"),
            Some(ArkitShape::EyeBlinkLeft)
        );
        assert_eq!(
            ArkitShape::from_name("mouthSmile_R"),
            Some(ArkitShape::MouthSmileRight)
        );
        assert_eq!(
            ArkitShape::from_name("EyeLookDown_L"),
            Some(ArkitShape::EyeLookDownLeft)
        );
        assert_eq!(
            ArkitShape::from_name("browInnerUp"),
            Some(ArkitShape::BrowInnerUp)
        );
        assert_eq!(
            ArkitShape::from_name("tongueOut"),
            Some(ArkitShape::TongueOut)
        );
        assert_eq!(ArkitShape::from_name("cheekPuff_L"), None);
        assert_eq!(ArkitShape::from_name(""), None);
        assert_eq!(ArkitShape::from_name("head"), None);
        assert_eq!(ArkitShape::from_name(&"a".repeat(1000)), None);
    }

    #[test]
    fn mapping_clamps_and_drops_non_finite() {
        let shapes = arkit_to_unified(&one_hot(ArkitShape::JawOpen, 3.0));
        assert_eq!(shapes.getu(U::JawOpen), 1.0);
        let shapes = arkit_to_unified(&one_hot(ArkitShape::JawOpen, -2.0));
        assert_eq!(shapes.getu(U::JawOpen), 0.0);
        let shapes = arkit_to_unified(&one_hot(ArkitShape::JawOpen, f32::NAN));
        assert_eq!(shapes.getu(U::JawOpen), 0.0);
        let shapes = arkit_to_unified(&one_hot(ArkitShape::JawOpen, f32::INFINITY));
        assert_eq!(shapes.getu(U::JawOpen), 0.0);
        let mut all = [5.0; ARKIT_COUNT];
        all[0] = f32::NEG_INFINITY;
        for value in arkit_to_unified(&all).iter() {
            assert!(value.is_finite());
            assert!((-1.0..=1.0).contains(value));
        }
    }

    #[test]
    fn mapping_routes_shapes_to_the_expected_unified_targets() {
        let cases = [
            (ArkitShape::EyeBlinkLeft, U::EyeClosedLeft),
            (ArkitShape::EyeBlinkRight, U::EyeClosedRight),
            (ArkitShape::BrowInnerUp, U::BrowInnerUpRight),
            (ArkitShape::CheekPuff, U::CheekPuffLeft),
            (ArkitShape::MouthSmileLeft, U::MouthCornerPullLeft),
            (ArkitShape::MouthFrownRight, U::MouthFrownRight),
            (ArkitShape::MouthRollUpper, U::LipSuckUpperLeft),
            (ArkitShape::MouthShrugLower, U::MouthRaiserLower),
            (ArkitShape::MouthClose, U::MouthClosed),
            (ArkitShape::TongueOut, U::TongueOut),
            (ArkitShape::MouthUpperUpRight, U::MouthUpperUpRight),
        ];
        for (shape, target) in cases {
            let shapes = arkit_to_unified(&one_hot(shape, 0.5));
            assert_eq!(shapes.getu(target), 0.5, "{shape:?} -> {target:?}");
        }
        let shapes = arkit_to_unified(&one_hot(ArkitShape::EyeLookInLeft, 1.0));
        assert_eq!(shapes.getu(U::EyeLeftX), 1.0);
        let shapes = arkit_to_unified(&one_hot(ArkitShape::EyeLookInRight, 1.0));
        assert_eq!(shapes.getu(U::EyeRightX), -1.0);
        let shapes = arkit_to_unified(&one_hot(ArkitShape::EyeLookUpLeft, 1.0));
        assert_eq!(shapes.getu(U::EyeY), 0.5);
    }

    #[test]
    fn nose_sneer_is_removed_from_upper_lip() {
        let mut weights = one_hot(ArkitShape::MouthUpperUpLeft, 0.6);
        weights[ArkitShape::NoseSneerLeft as usize] = 0.8;
        let shapes = arkit_to_unified(&weights);
        assert_eq!(shapes.getu(U::MouthUpperUpLeft), 0.0);
        assert_eq!(shapes.getu(U::NoseSneerLeft), 0.8);
    }
}
