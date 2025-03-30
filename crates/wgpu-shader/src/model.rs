use std::ops::Range;

use glam::Vec4;

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
pub const CUBE: [Color4; 8] = [
    Color4::new(Vec4::new(-0.5, -0.5, -0.5, 1.0), V4Y),
    Color4::new(Vec4::new(0.5, -0.5, -0.5, 1.0), V4X),
    Color4::new(Vec4::new(-0.5, -0.5, 0.5, 1.0), V4Z),
    Color4::new(Vec4::new(0.5, -0.5, 0.5, 1.0), V4X),
    Color4::new(Vec4::new(-0.5, 0.5, -0.5, 1.0), V4Y),
    Color4::new(Vec4::new(0.5, 0.5, -0.5, 1.0), V4X),
    Color4::new(Vec4::new(-0.5, 0.5, 0.5, 1.0), V4Z),
    Color4::new(Vec4::new(0.5, 0.5, 0.5, 1.0), V4X),
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

/// [CUBE]の長さを指定して頂点データを生成する
pub fn cube(length: f32) -> [Color4; 8] {
    let mut cube = CUBE;
    for c in cube.iter_mut() {
        c.position *= length;
        c.position.w = 1.0;
    }
    cube
}
