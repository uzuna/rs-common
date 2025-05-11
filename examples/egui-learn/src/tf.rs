#[cfg(test)]
mod tests {
    use gltf::accessor::Accessor;

    fn read_buffer<'a>(buffer: &'a [u8], a: &'a Accessor<'_>) -> &'a [u8] {
        let length = a.size() * a.count();
        let start = a.offset();
        let end = start + length;
        buffer.get(start..end).expect("Buffer out of range")
    }

    fn traverse_node(buffer: &[u8], node: &gltf::Node) {
        println!(
            "Node: {:?}, {:?}, {:?}",
            node.name(),
            node.index(),
            node.transform()
        );

        // メッシュデータへのアクセス
        if let Some(mesh) = node.mesh() {
            println!("  Mesh: {:?}", mesh.name());
            for primitive in mesh.primitives() {
                println!("    Primitive: {:?}", primitive.mode());
                primitive.attributes().for_each(|(semantic, _)| {
                    println!("    Attribute: {:?}", semantic);
                    if let Some(a) = primitive.get(&semantic) {
                        let buf = read_buffer(buffer, &a);
                        println!(
                            "      Detail: {:?} {:?} {:?}",
                            a.data_type(),
                            a.dimensions(),
                            buf.len(),
                        );
                    }
                });
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
        let l = ["testdata/box.glb", "testdata/duck.glb"];

        for path in l.iter() {
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
            // g.images()
            //     .for_each(|image| println!("Image: {:?}", image.source()));
            // g.textures()
            //     .for_each(|texture| println!("Texture: {:?}", texture.sampler()));
        }
    }
}
