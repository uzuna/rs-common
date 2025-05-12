use std::vec;

use gltf::Accessor;
use wgpu_shader::{
    graph::{ModelNodeImpl, Trs},
    prelude::glam::{self, Vec2, Vec3},
};

pub enum GltfSlot {
    None,
    Draw(Mesh),
}

pub struct Mesh {
    pub name: String,
    pub primitives: Vec<Primitive>,
}

impl Mesh {
    fn new(buffer: &[u8], mesh: gltf::Mesh) -> Self {
        let mut primitives = Vec::with_capacity(mesh.primitives().len());
        for primitive in mesh.primitives() {
            primitives.push(Primitive::parse(buffer, primitive));
        }
        Self {
            name: Mesh::parse_name(&mesh),
            primitives,
        }
    }

    fn parse_name(mesh: &gltf::Mesh) -> String {
        mesh.name()
            .map(|s| s.to_string())
            .unwrap_or(format!("mesh_id_{}", mesh.index()))
    }
}

pub struct Primitive {
    pub primitive: wgpu::PrimitiveTopology,
    pub index: Option<Vec<u16>>,
    pub position: Option<Vec<glam::Vec3>>,
    pub normal: Option<Vec<glam::Vec3>>,
    pub color: Option<Vec<glam::Vec3>>,
    pub texcoord: Option<Vec<glam::Vec2>>,
    pub material: String,
}

impl Primitive {
    fn parse(buffer: &[u8], prim: gltf::Primitive) -> Self {
        let material = prim.material();
        let mut p = Primitive {
            primitive: wgpu::PrimitiveTopology::TriangleList,
            index: None,
            position: None,
            normal: None,
            color: None,
            texcoord: None,
            material: Material::parse_name(&material),
        };
        // indexがある場合は、インデックスバッファを取得
        if let Some(a) = prim.indices() {
            let buf = read_buffer(buffer, &a);
            p.index = Some(parse_buffer::<u16>(buf, a.size(), a.count()));
        }

        // attributesを取得
        prim.attributes().for_each(|(semantic, a)| match semantic {
            gltf::mesh::Semantic::Positions => {
                let buf = read_buffer(buffer, &a);
                p.position = Some(parse_buffer::<Vec3>(buf, a.size(), a.count()));
            }
            gltf::mesh::Semantic::Normals => {
                let buf = read_buffer(buffer, &a);
                p.normal = Some(parse_buffer::<Vec3>(buf, a.size(), a.count()));
            }
            gltf::mesh::Semantic::Colors(_) => {
                // TODO: Vec4カラーに対応
                let buf = read_buffer(buffer, &a);
                p.color = Some(parse_buffer::<Vec3>(buf, a.size(), a.count()));
            }
            gltf::mesh::Semantic::TexCoords(_) => {
                let buf = read_buffer(buffer, &a);
                p.texcoord = Some(parse_buffer::<Vec2>(buf, a.size(), a.count()));
            }
            _ => {}
        });
        p
    }
}

pub struct Material {
    pub name: String,
    pub base_color: glam::Vec4,
    pub metallic: f32,
    pub roughness: f32,
}

impl Material {
    fn parse_name(material: &gltf::Material) -> String {
        material
            .name()
            .map(|s| s.to_string())
            .unwrap_or(format!("material_id_{}", material.index().unwrap()))
    }
}

pub struct GraphBuilder {
    graph: wgpu_shader::graph::ModelGraph<wgpu_shader::graph::ModelNode<GltfSlot>>,
}

impl GraphBuilder {
    pub fn new() -> Self {
        Self {
            graph: wgpu_shader::graph::ModelGraph::new(),
        }
    }

    // GLTFノードを解析して必要なデータを読み出す
    fn parse_node(buffer: &[u8], node: &gltf::Node) -> GltfSlot {
        if let Some(mesh) = node.mesh() {
            GltfSlot::Draw(Mesh::new(buffer, mesh))
        } else {
            GltfSlot::None
        }
    }

    /// ノードを再帰的にたどる
    fn traverse_inner(
        &mut self,
        buffer: &[u8],
        parent: Option<&str>,
        node: &gltf::Node,
        f: &impl Fn(&[u8], &gltf::Node) -> GltfSlot,
    ) -> anyhow::Result<()> {
        let name = node
            .name()
            .map(|s| s.to_string())
            .unwrap_or(format!("id_{}", node.index()));
        let n = wgpu_shader::graph::ModelNode::new(
            name.clone(),
            gltf_trans_to_trs(node.transform()),
            f(buffer, node),
        );
        self.graph.add_node(parent, n)?;
        for child in node.children() {
            self.traverse_inner(buffer, Some(name.as_str()), &child, f)?;
        }
        Ok(())
    }

    /// GLBファイルからグラフを構築する
    pub fn build(
        mut self,
        glb: &gltf::Glb,
    ) -> anyhow::Result<wgpu_shader::graph::ModelGraph<wgpu_shader::graph::ModelNode<GltfSlot>>>
    {
        let g = gltf::Gltf::from_slice(&glb.json)?;
        let buffer = glb.bin.as_ref().unwrap();
        for scene in g.scenes() {
            for node in scene.nodes() {
                self.traverse_inner(buffer.as_ref(), None, &node, &Self::parse_node)?;
            }
        }
        Ok(self.graph)
    }
}
// GLTFのバッファを取得する
fn read_buffer<'a>(buffer: &'a [u8], a: &'a Accessor<'_>) -> &'a [u8] {
    let length = a.size() * a.count();
    let start = a.offset();
    let end = start + length;
    buffer.get(start..end).expect("Buffer out of range")
}

// バッファを指定サイズのスライスに分割してVecに変換する
fn parse_buffer<T>(buffer: &[u8], size: usize, count: usize) -> Vec<T>
where
    T: bytemuck::Pod,
{
    let mut v = Vec::with_capacity(count);
    for i in 0..count {
        let start = size * i;
        let end = start + size;
        let slice = buffer.get(start..end).expect("Buffer out of range");
        let value: T = bytemuck::pod_read_unaligned(slice);
        v.push(value);
    }
    v
}

// GLTFのTransformをwgpu_shaderのTRSに変換する
fn gltf_trans_to_trs(trans: gltf::scene::Transform) -> Trs {
    let decon = trans.decomposed();

    Trs {
        translation: glam::Vec3::from(decon.0),
        rotation: glam::Quat::from_array(decon.1),
        scale: glam::Vec3::from(decon.2),
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use gltf::{accessor::Accessor, buffer::View};
    use wgpu_shader::graph::ModelNodeImpl;

    use crate::tf::{GltfSlot, GraphBuilder};

    fn read_buffer<'a>(buffer: &'a [u8], a: &'a Accessor<'_>) -> &'a [u8] {
        let length = a.size() * a.count();
        let start = a.offset();
        let end = start + length;
        buffer.get(start..end).expect("Buffer out of range")
    }

    fn read_buffer_view<'a>(buffer: &'a [u8], a: &'a View<'_>) -> &'a [u8] {
        let length = a.length();
        let start = a.offset();
        let end = start + length;
        buffer.get(start..end).expect("Buffer out of range")
    }

    fn traverse_node(buffer: &[u8], node: &gltf::Node) {
        println!(
            "  Node: {:?}, {:?}, {:?}",
            node.name(),
            node.index(),
            node.transform()
        );

        // メッシュデータへのアクセス
        if let Some(mesh) = node.mesh() {
            println!("    Mesh: {:?}", mesh.name());
            for primitive in mesh.primitives() {
                println!("      Primitive: {:?}", primitive.mode());
                if let Some(index) = primitive.indices() {
                    println!("        Indices: {:?}", index.count());
                    let buf = read_buffer(buffer, &index);
                    println!(
                        "          Detail: {:?} {:?} {:?}",
                        index.data_type(),
                        index.dimensions(),
                        buf.len(),
                    );
                }
                primitive.attributes().for_each(|(semantic, _)| {
                    println!("      Attribute: {:?}", semantic);
                    if let Some(a) = primitive.get(&semantic) {
                        let buf = read_buffer(buffer, &a);
                        println!(
                            "        Detail: {:?} {:?} {:?}",
                            a.data_type(),
                            a.dimensions(),
                            buf.len(),
                        );
                    }
                });
                let material = primitive.material();
                println!("      Material: {:?}", material.name());
                let base_color = material.pbr_metallic_roughness().base_color_factor();
                println!("        Base Color: {:?}", base_color);
                if let Some(texture) = material.pbr_metallic_roughness().base_color_texture() {
                    let texture = texture.texture();
                    let image = texture.source();
                    println!("        Image: {:?}", image.index());
                }
            }
        }

        for child in node.children() {
            traverse_node(buffer, &child);
        }
    }

    #[test]
    fn test_load_glb() {
        use std::path::PathBuf;
        // https://github.com/KhronosGroup/glTF-Sample-Models/blob/main/2.0/Box/glTF/Box.gltf
        let l = ["testdata/box.glb", "testdata/box2.glb", "testdata/duck.glb"];

        for path in l.iter() {
            println!("Loading: {}", path);
            let path = PathBuf::from(path);
            let glb = gltf::Glb::from_reader(std::fs::File::open(path).unwrap()).unwrap();
            let g = gltf::Gltf::from_slice(&glb.json).unwrap();

            // ノードを取得
            let buffer = glb.bin.unwrap();
            for scene in g.scenes() {
                for node in scene.nodes() {
                    traverse_node(buffer.as_ref(), &node);
                }
            }
            g.images().for_each(|image| match image.source() {
                gltf::image::Source::View { view, mime_type } => {
                    println!("  Image: {:?} {:?}", view.index(), mime_type);
                    let view = g.views().nth(view.index()).unwrap();
                    let buf = read_buffer_view(buffer.as_ref(), &view);
                    println!("    Length: {}", buf.len());
                }
                gltf::image::Source::Uri { uri, mime_type } => {
                    println!("  Image: {:?} {:?}", uri, mime_type);
                }
            });
        }
    }

    #[test]
    fn test_build_graph() -> anyhow::Result<()> {
        const PATH: &str = "testdata/box2.glb";
        let path = PathBuf::from(PATH);
        let glb = gltf::Glb::from_reader(std::fs::File::open(path).unwrap()).unwrap();

        let builder = GraphBuilder::new();
        let g = builder.build(&glb)?;
        for node in g.iter() {
            println!("Node: {:?} {:?}", node.name(), node.trs());
            let v = node.value();
            match v {
                GltfSlot::Draw(draw) => {
                    println!("  Draw: {:?}", draw.name);
                    for primitive in &draw.primitives {
                        println!("    Primitive: {:?}", primitive.primitive);
                        if let Some(index) = &primitive.index {
                            println!("    Index: {:?}", index.len());
                        }
                        if let Some(position) = &primitive.position {
                            println!("    Position: {:?}", position.len());
                        }
                        if let Some(normal) = &primitive.normal {
                            println!("    Normal: {:?}", normal.len());
                        }
                        if let Some(color) = &primitive.color {
                            println!("    Color: {:?}", color.len());
                        }
                        if let Some(texcoord) = &primitive.texcoord {
                            println!("    TexCoord: {:?}", texcoord.len());
                        }
                        println!("    Material: {:?}", primitive.material);
                    }
                }
                GltfSlot::None => {}
            }
        }
        Ok(())
    }
}
