//! A colour written as red, green and blue, as the nearest whole hue,
//! lightness and saturation ReGIS takes (script-api.md section 6.1).
//! ReGIS sets a colour only so (`S(M n(H L S))`, the manual's chapter 2,
//! "Output Mapping"), with its own hue circle: 0 is blue, 120 red and 240
//! green.

/// The most a red, green or blue part is.
const PART_MAX: i32 = 255;
/// A sixth of the hue circle, in degrees: the width of one sector between
/// a primary and a secondary colour.
const SECTOR: i32 = 60;
/// The whole hue circle, in degrees.
const CIRCLE: i32 = 360;
/// Where red is on ReGIS's hue circle, which starts at blue; on the
/// circle that starts at red it is 0.
const RED_HUE: i32 = 120;
/// A whole percentage's ceiling.
const PERCENT: i32 = 100;

/// `numerator / denominator`, to the nearest whole number; the
/// denominator is positive.
#[expect(
    clippy::arithmetic_side_effects,
    reason = "every value is a product of byte-sized parts and small constants, far inside an i32, and the divisor is positive"
)]
const fn nearest(numerator: i32, denominator: i32) -> i32 {
    (2 * numerator + denominator).div_euclid(2 * denominator)
}

/// The hue (0 to 359), lightness and saturation (0 to 100) nearest the
/// colour `rgb`, 8 bits each of red, green and blue from the top.
#[must_use]
#[expect(
    clippy::arithmetic_side_effects,
    reason = "every value is a product of byte-sized parts and small constants, far inside an i32, and each divisor is positive where it is used"
)]
pub(super) const fn hls_of_rgb(rgb: u32) -> (i32, i32, i32) {
    let red = ((rgb >> 16) & 0xFF).cast_signed();
    let green = ((rgb >> 8) & 0xFF).cast_signed();
    let blue = (rgb & 0xFF).cast_signed();
    let most = if red > green {
        if red > blue { red } else { blue }
    } else if green > blue {
        green
    } else {
        blue
    };
    let least = if red < green {
        if red < blue { red } else { blue }
    } else if green < blue {
        green
    } else {
        blue
    };
    let sum = most + least;
    let spread = most - least;
    let lightness = nearest(sum * PERCENT, 2 * PART_MAX);
    if spread == 0 {
        return (0, lightness, 0);
    }
    // The double-cone model the manual's HLS is: saturation is the spread
    // against the most it could be at this lightness.
    let room = if sum <= PART_MAX { sum } else { 2 * PART_MAX - sum };
    let saturation = nearest(spread * PERCENT, room);
    let from_red = if most == red {
        nearest((green - blue) * SECTOR, spread)
    } else if most == green {
        2 * SECTOR + nearest((blue - red) * SECTOR, spread)
    } else {
        4 * SECTOR + nearest((red - green) * SECTOR, spread)
    };
    ((from_red + RED_HUE).rem_euclid(CIRCLE), lightness, saturation)
}
