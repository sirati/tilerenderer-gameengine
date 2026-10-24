//! Readable status text without game files, font files, or cursor atlas art.
use render_desc::{Compiled, Description, ImageLoader};
use tile_render::{Headless, TileRenderer};
use tile_viewer::host_status::{self, Anchor, Responsiveness};

fn builtin()->Compiled {
    struct NoFiles;
    impl ImageLoader for NoFiles {
        fn load(&self,_:&str,_:&toml::Table)->Result<Vec<tile_render::RgbaImage>,String> {Err("missing game art".into())}
        fn font(&self,_:&str)->Result<Vec<u8>,String> {Err("missing game font".into())}
    }
    Description::parse("[grid]\nlayout=\"orthogonal\"\nstep=[1,1]\n[fonts.game]\nsource=\"missing.ttf\"\n")
        .unwrap().compile_layout(&NoFiles,16).unwrap()
}

#[test]
fn gpu_host_warning_has_readable_ink_with_only_the_embedded_vector_face() {
    let gpu=Headless::new().expect("RADV GPU");
    let compiled=builtin();
    assert!(compiled.font("game").is_none());
    assert!(compiled.font(render_desc::FALLBACK_FONT).is_some());
    let world=compiled.world(1,1,false,false);
    let mut renderer=TileRenderer::new(&gpu.device,wgpu::TextureFormat::Rgba8Unorm, tile_viewer::SHADERS);
    renderer.set_world(&gpu.device,&world);
    for (window,controls,scale) in [((300,200),[0,0,149,54],(2,1)),((900,600),[0,0,224,81],(3,2)),
        ((1600,900),[800,0,298,108],(2,1)),((1600,900),[0,0,596,216],(4,1)),((640,480),[0,0,596,216],(32,1))] {
      for state in [Responsiveness::Busy,Responsiveness::Stuck] {
        let (camera,commands)=host_status::compose(&compiled,state,Anchor::Beside(controls,scale),window,0.0,0).unwrap();
        assert_eq!(commands.iter().filter(|s|s.text.is_some()).count(),state.text().unwrap().len());
        let backgrounds:Vec<_>=commands.iter().filter(|s|s.text.is_none()).copied().collect();
        renderer.set_markers(&gpu.device,&gpu.queue,&backgrounds);
        let without=gpu.render(&mut renderer,&camera,0,[0,0,0,255]);
        renderer.set_markers(&gpu.device,&gpu.queue,&commands);
        let readable=gpu.render(&mut renderer,&camera,0,[0,0,0,255]);
        let ink=readable.pixels.iter().zip(&without.pixels).filter(|(a,b)|a!=b).count();
        assert!(ink>200,"{state:?}: {ink} actual GPU glyph pixels, not a background-only group");
        let mut oracle=world.clone();oracle.free.extend(commands);
        let want=tile_render::reference::render(&oracle,&tile_render::AtlasBuilder::new(16).finish(),&camera,0,[0,0,0,255]);
        let most=readable.pixels.iter().zip(&want.pixels).flat_map(|(a,b)|a.iter().zip(b).map(|(a,b)|a.abs_diff(*b))).max().unwrap();
        assert!(most<=1,"glyph and painter order reference, maximum channel difference {most}");
        // The original controls rectangle is completely untouched.
        assert!((controls[1]..controls[1]+controls[3]).all(|y|(controls[0]..controls[0]+controls[2]).all(|x|readable.get(x as u32,y as u32)==[0,0,0,255])));
      }
    }
}
