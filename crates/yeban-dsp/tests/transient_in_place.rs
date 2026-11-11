use yeban_dsp::shaping::{TransientParams, TransientShaper};

#[test]
fn transient_shaper_process_in_place_matches_process() {
    let mut shaper1 = TransientShaper::new();
    let mut shaper2 = TransientShaper::new();
    let sr = 48_000.0;
    let params = TransientParams {
        attack_amt: 0.5,
        sustain_amt: -0.3,
    };

    let in_l = vec![0.1f32; 256];
    let in_r = vec![0.2f32; 256];
    let mut out_l = vec![0.0f32; 256];
    let mut out_r = vec![0.0f32; 256];

    shaper1.process(&in_l, &in_r, &mut out_l, &mut out_r, params, sr);

    let mut in_place_l = in_l.clone();
    let mut in_place_r = in_r.clone();
    shaper2.process_in_place(&mut in_place_l, &mut in_place_r, params, sr);

    assert_eq!(in_place_l, out_l);
    assert_eq!(in_place_r, out_r);
}

#[test]
fn transient_shaper_in_place_neutral_is_exact() {
    let mut shaper = TransientShaper::new();
    let sr = 48_000.0;
    let params = TransientParams::default();

    let mut in_l = vec![0.35f32; 128];
    let mut in_r = vec![-0.45f32; 128];
    let orig_l = in_l.clone();
    let orig_r = in_r.clone();

    shaper.process_in_place(&mut in_l, &mut in_r, params, sr);

    assert_eq!(in_l, orig_l);
    assert_eq!(in_r, orig_r);
}
