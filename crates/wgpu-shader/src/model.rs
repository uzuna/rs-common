use std::ops::Range;

use glam::{Vec3, Vec4};

use crate::{texture, types::vertex::Color4};

/// マテリアル情報
pub struct Material {
    pub name: String,
    pub texture: texture::Texture,
    pub bg: wgpu::BindGroup,
}

/// メッシュ情報
pub struct Mesh {
    pub name: String,
    pub vertex_buffer: wgpu::Buffer,
    pub index_buffer: wgpu::Buffer,
    pub num_indices: u32,
    pub material: usize,
}

pub trait DrawModel<'a> {
    fn draw_mesh(&mut self, mesh: &'a Mesh);
    fn draw_mesh_instanced(&mut self, mesh: &'a Mesh, instances: Range<u32>);
}

impl<'a> DrawModel<'a> for wgpu::RenderPass<'a> {
    fn draw_mesh(&mut self, mesh: &'a Mesh) {
        self.draw_mesh_instanced(mesh, 0..1);
    }

    fn draw_mesh_instanced(&mut self, mesh: &'a Mesh, instances: Range<u32>) {
        self.set_vertex_buffer(0, mesh.vertex_buffer.slice(..));
        self.set_index_buffer(mesh.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
        self.draw_indexed(0..mesh.num_indices, 0, instances);
    }
}

const ROOT: Vec4 = Vec4::new(0.0, 0.0, 0.0, 1.0);
const V4X: Vec4 = Vec4::new(1.0, 0.0, 0.0, 1.0);
const V4Y: Vec4 = Vec4::new(0.0, 1.0, 0.0, 1.0);
const V4Z: Vec4 = Vec4::new(0.0, 0.0, 1.0, 1.0);
/// 右手系左手系確認用の単位ベクトルの頂点データ
pub const HAND4: [Color4; 6] = [
    Color4::new(ROOT, V4X),
    Color4::new(V4X, V4X),
    Color4::new(ROOT, V4Y),
    Color4::new(V4Y, V4Y),
    Color4::new(ROOT, V4Z),
    Color4::new(V4Z, V4Z),
];

/// [HAND4]の長さを指定して頂点データを生成する
pub fn hand4(length: f32) -> [Color4; 6] {
    let mut hand = HAND4;
    for h in hand.iter_mut() {
        h.position *= length;
        h.position.w = 1.0;
    }
    hand
}

/// 幅1.0の立方体の頂点データ
/// 前方が赤、後方上が青、後方下が緑となっている
pub const CUBE: [Vec4; 8] = [
    Vec4::new(-0.5, -0.5, -0.5, 1.0),
    Vec4::new(0.5, -0.5, -0.5, 1.0),
    Vec4::new(-0.5, -0.5, 0.5, 1.0),
    Vec4::new(0.5, -0.5, 0.5, 1.0),
    Vec4::new(-0.5, 0.5, -0.5, 1.0),
    Vec4::new(0.5, 0.5, -0.5, 1.0),
    Vec4::new(-0.5, 0.5, 0.5, 1.0),
    Vec4::new(0.5, 0.5, 0.5, 1.0),
];

/// [CUBE]のインデックスデータ
pub const CUBE_INDEX: [u16; 36] = [
    0, 1, 2, 1, 3, 2, // left
    6, 4, 2, 4, 0, 2, // front
    6, 7, 4, 7, 5, 4, // right
    4, 5, 0, 5, 1, 0, // bottom
    5, 7, 1, 7, 3, 1, // back
    7, 6, 3, 6, 2, 3, // top
];

pub const CUBE_COLOR: [Vec3; 6] = [
    Vec3::new(200.0, 70.0, 120.0),  // left
    Vec3::new(70.0, 200.0, 210.0),  // front
    Vec3::new(80.0, 70.0, 200.0),   // right
    Vec3::new(90.0, 130.0, 110.0),  // bottom
    Vec3::new(160.0, 160.0, 220.0), // back
    Vec3::new(200.0, 200.0, 70.0),  //top
];

/// [CUBE]の長さを指定して頂点データを生成する
pub fn cube(length: f32) -> Vec<Color4> {
    let mut cube = Vec::with_capacity(36);
    for (index, c) in CUBE_INDEX.into_iter().enumerate() {
        let pos = CUBE[c as usize] * length;
        let color = CUBE_COLOR[index / 6] / 255.0;
        cube.push(Color4::new(pos, Vec4::new(color.x, color.y, color.z, 1.0)));
    }
    cube
}

// XY平面の四角形の頂点データ
pub fn rect(length: f32) -> [Color4; 4] {
    let l = length / 2.0;
    [
        Color4::new(Vec4::new(-l, -l, 0.0, 1.0), V4Y),
        Color4::new(Vec4::new(l, -l, 0.0, 1.0), V4X),
        Color4::new(Vec4::new(-l, l, 0.0, 1.0), V4Y),
        Color4::new(Vec4::new(l, l, 0.0, 1.0), V4X),
    ]
}

/// [rect]のインデックスデータ
pub const RECT_INDEX: [u16; 6] = [0, 1, 2, 1, 3, 2];
