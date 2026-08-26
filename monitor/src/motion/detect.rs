use anyhow::{Context, Result, bail};

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Roi {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Roi {
    pub(crate) fn parse(value: &str) -> Result<Self> {
        let mut parts = value.split(',');
        let parse_part = |part: Option<&str>, name: &str| -> Result<f32> {
            let part = part
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .with_context(|| format!("MONITOR_MOTION_ROI must be x,y,w,h (missing {name})"))?;
            part.parse::<f32>()
                .with_context(|| format!("MONITOR_MOTION_ROI {name} is not a number"))
        };

        let roi = Self {
            x: parse_part(parts.next(), "x")?,
            y: parse_part(parts.next(), "y")?,
            w: parse_part(parts.next(), "w")?,
            h: parse_part(parts.next(), "h")?,
        };
        if parts.next().is_some() {
            bail!("MONITOR_MOTION_ROI must be x,y,w,h");
        }
        for (name, value) in [("x", roi.x), ("y", roi.y), ("w", roi.w), ("h", roi.h)] {
            if !value.is_finite() || !(0.0..=1.0).contains(&value) {
                bail!("MONITOR_MOTION_ROI {name} must be between 0 and 1");
            }
        }
        if roi.w == 0.0 || roi.h == 0.0 {
            bail!("MONITOR_MOTION_ROI width and height must be greater than zero");
        }
        if roi.x + roi.w > 1.0 + f32::EPSILON || roi.y + roi.h > 1.0 + f32::EPSILON {
            bail!("MONITOR_MOTION_ROI must stay within the unit square");
        }
        Ok(roi)
    }

    fn pixel_bounds(self, width: usize, height: usize) -> (usize, usize, usize, usize) {
        let x0 = ((self.x * width as f32).floor() as usize).min(width);
        let y0 = ((self.y * height as f32).floor() as usize).min(height);
        let x1 = (((self.x + self.w) * width as f32).ceil() as usize).clamp(x0, width);
        let y1 = (((self.y + self.h) * height as f32).ceil() as usize).clamp(y0, height);
        (x0, y0, x1, y1)
    }
}

pub(crate) fn motion_score(
    previous: &[u8],
    current: &[u8],
    width: usize,
    height: usize,
    pixel_floor: u8,
    roi: Option<Roi>,
) -> f32 {
    let expected = width.saturating_mul(height);
    if previous.len() != expected || current.len() != expected || expected == 0 {
        return 0.0;
    }

    let (x0, y0, x1, y1) = roi.map_or((0, 0, width, height), |roi| roi.pixel_bounds(width, height));
    let mut changed = 0u32;
    let mut total = 0u32;
    for y in y0..y1 {
        let row = y * width;
        for x in x0..x1 {
            total += 1;
            if previous[row + x].abs_diff(current[row + x]) > pixel_floor {
                changed += 1;
            }
        }
    }
    if total == 0 {
        0.0
    } else {
        changed as f32 / total as f32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_roi_box() {
        assert_eq!(
            Roi::parse("0,0,0.6,1").unwrap(),
            Roi {
                x: 0.0,
                y: 0.0,
                w: 0.6,
                h: 1.0
            }
        );
        assert!(Roi::parse("").is_err());
        assert!(Roi::parse("0,0,1.2,1").is_err());
        assert!(Roi::parse("0.5,0,0.6,1").is_err());
        assert!(Roi::parse("0,0,0,1").is_err());
    }

    #[test]
    fn identical_frames_score_zero() {
        let frame = vec![40; 8];
        assert_eq!(motion_score(&frame, &frame, 4, 2, 25, None), 0.0);
    }

    #[test]
    fn reports_fraction_of_changed_pixels() {
        let previous = vec![0; 4];
        let mut current = vec![0; 4];
        current[0] = 255;
        current[1] = 255;
        assert!((motion_score(&previous, &current, 2, 2, 25, None) - 0.5).abs() < f32::EPSILON);
    }

    #[test]
    fn noise_below_floor_is_ignored() {
        let previous = vec![10; 4];
        let current = vec![20; 4];
        assert_eq!(motion_score(&previous, &current, 2, 2, 25, None), 0.0);
    }

    #[test]
    fn roi_ignores_motion_outside_the_box() {
        let previous = vec![0; 4];
        let mut current = vec![0; 4];
        current[1] = 255;
        current[3] = 255;
        let roi = Roi::parse("0,0,0.5,1").unwrap();
        assert_eq!(motion_score(&previous, &current, 2, 2, 25, Some(roi)), 0.0);
    }
}
