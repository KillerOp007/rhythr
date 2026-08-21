//! Debug tool: reads {"outline":[[x,y]..],"control":[{x,y,smooth}..]|null,
//! "hollow":t|null} as JSON on stdin and prints the NotePlan as JSON.
//! Lets the screenshot mock draw the REAL backend geometry, control-point
//! flattening included, instead of a port of it.
fn main() {
    let mut input = String::new();
    std::io::Read::read_to_string(&mut std::io::stdin(), &mut input).expect("stdin");
    let v: serde_json::Value = serde_json::from_str(&input).expect("json");
    let control: Option<Vec<rhythia_render::designer::ControlPoint>> = v
        .get("control")
        .filter(|c| c.is_array())
        .map(|c| serde_json::from_value(c.clone()).expect("control"));
    let outline: Vec<[f32; 2]> = match control {
        Some(c) if c.len() >= 3 => rhythia_render::designer::flatten_controls(&c),
        _ => v["outline"]
            .as_array()
            .expect("outline")
            .iter()
            .map(|p| {
                [
                    p[0].as_f64().unwrap_or(0.0) as f32,
                    p[1].as_f64().unwrap_or(0.0) as f32,
                ]
            })
            .collect(),
    };
    let hollow = v["hollow"].as_f64().map(|t| t as f32);
    match rhythia_render::designer::plan_note(&outline, hollow) {
        Ok(plan) => println!("{}", serde_json::to_string(&plan).unwrap()),
        Err(_) => println!("{{\"error\":\"outline encloses no area\"}}"),
    }
}
