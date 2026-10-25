//! Import layering. An SVG with several colours arrives as one layer per
//! colour, in paint order. The game shows one tint per note, so a colour
//! can only be told apart by material: a later colour that lies on
//! material already there becomes a Cut (the figure on a badge, visible as
//! a hole in the editor and in the game), a later colour on empty ground
//! adds (stripes side by side stay a full silhouette).

use crate::kit::doc::Mode;
use crate::kit::geom::{
    boolean, clean, is_empty, signed_area, Contour, FillRule, OverlayRule, Shapes,
};
use crate::kit::KitError;

/// The share of a layer's area that must already be material for the
/// layer to cut instead of add.
pub const CUT_SHARE: f32 = 0.5;

fn area(shapes: &Shapes) -> f32 {
    // Holes run clockwise and come out negative, so the sum is the net area.
    shapes.iter().flatten().map(|c| signed_area(c)).sum()
}

/// Add or Cut for every layer, in order. Layers are raw contours as the
/// importer hands them (oriented, one colour each).
pub fn layer_modes(layers: &[Vec<Contour>]) -> Result<Vec<Mode>, KitError> {
    let mut modes = Vec::with_capacity(layers.len());
    let mut material: Shapes = Vec::new();
    for contours in layers {
        let region = clean(contours, FillRule::NonZero)?;
        let own = area(&region);
        let covered = if is_empty(&material) || own <= 0.0 {
            0.0
        } else {
            area(&boolean(&region, &material, OverlayRule::Intersect)?)
        };
        if covered > CUT_SHARE * own {
            modes.push(Mode::Cut);
            material = boolean(&material, &region, OverlayRule::Difference)?;
        } else {
            modes.push(Mode::Add);
            material = if is_empty(&material) {
                region
            } else {
                boolean(&material, &region, OverlayRule::Union)?
            };
        }
    }
    Ok(modes)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn square(cx: f32, cy: f32, half: f32) -> Contour {
        vec![
            [cx - half, cy - half],
            [cx + half, cy - half],
            [cx + half, cy + half],
            [cx - half, cy + half],
        ]
    }

    #[test]
    fn a_figure_on_a_ground_cuts_and_a_figure_on_the_figure_adds_again() {
        // Badge, face on it, eye on the face: the face is cut out of the
        // badge, the eye lies where nothing is left and adds an island.
        let layers = vec![
            vec![square(0.0, 0.0, 1.0)],
            vec![square(0.0, 0.0, 0.5)],
            vec![square(0.0, 0.0, 0.1)],
        ];
        assert_eq!(
            layer_modes(&layers).unwrap(),
            vec![Mode::Add, Mode::Cut, Mode::Add]
        );
    }

    #[test]
    fn colours_side_by_side_all_add() {
        let layers = vec![
            vec![square(-0.6, 0.0, 0.3)],
            vec![square(0.0, 0.0, 0.3)],
            vec![square(0.6, 0.0, 0.3)],
        ];
        assert_eq!(
            layer_modes(&layers).unwrap(),
            vec![Mode::Add, Mode::Add, Mode::Add]
        );
    }

    #[test]
    fn a_layer_half_on_material_adds_until_it_lies_mostly_on_it() {
        let ground = vec![square(0.0, 0.0, 0.5)];
        // 40 percent on the ground: adds. 80 percent: cuts.
        let mostly_off = vec![square(0.6, 0.0, 0.5)];
        let mostly_on = vec![square(0.2, 0.0, 0.5)];
        assert_eq!(
            layer_modes(&[ground.clone(), mostly_off]).unwrap(),
            vec![Mode::Add, Mode::Add]
        );
        assert_eq!(
            layer_modes(&[ground, mostly_on]).unwrap(),
            vec![Mode::Add, Mode::Cut]
        );
    }

    #[test]
    fn empty_layers_add_harmlessly() {
        let layers = vec![Vec::new(), vec![square(0.0, 0.0, 1.0)]];
        assert_eq!(layer_modes(&layers).unwrap(), vec![Mode::Add, Mode::Add]);
    }
}
