//! Cabinetシーングラフを考える
//!
//! Rootの下にCabinetというひとくくりがある。
//! 外形をしめすメッシュの他に、引き出しオブジェクトがある作り
//! 今回は概念名はCabinet -> Drawer(引き出し)
//! 実体はCubeNode(TRS+Colorで頂点を変形)とその親子関係となる。
//!
//! 実装として必要なのは Nodeが参照する頂点を変えられること
//!
//! 今回はの作成順序
//! 引き出しNode -> CubeNode
//! 引き出しをまとめるtCabinetNode
//! Cabinetの移動
//!
//! JSの実装はlocalのMat4を連続的に書き換えている。
//!
//! SceneGraphNode
//! - Name: 識別子
//! - Children: 子の座標
//! - local: ローカル上座標
//! - world: ワールド上の座標
//!
//! update_world -> 親のワールご位置の更新を伝播させる
//! 親のworldに対してlocalを適用すると、自身のworldを得られる
//! 自身のワールドを更に子に適用したら良い

/// 各ノードのTRS操作
pub struct Trs {
    pub translation: glam::Vec3,
    pub rotation: glam::Quat,
    pub scale: glam::Vec3,
}

impl Trs {
    pub fn new(translation: glam::Vec3, rotation: glam::Quat, scale: glam::Vec3) -> Self {
        Self {
            translation,
            rotation,
            scale,
        }
    }

    pub fn with_tr(translation: glam::Vec3, rotation: glam::Quat) -> Self {
        Self::new(translation, rotation, glam::Vec3::ONE)
    }

    pub fn with_t(translation: glam::Vec3) -> Self {
        Self::new(translation, glam::Quat::IDENTITY, glam::Vec3::ONE)
    }

    pub fn set_translation(&mut self, translation: glam::Vec3) {
        self.translation = translation;
    }

    pub fn set_rot_x(&mut self, angle: f32) {
        self.rotation = glam::Quat::from_rotation_x(angle);
    }

    pub fn set_rot_y(&mut self, angle: f32) {
        self.rotation = glam::Quat::from_rotation_y(angle);
    }

    pub fn set_rot_z(&mut self, angle: f32) {
        self.rotation = glam::Quat::from_rotation_z(angle);
    }

    pub fn set_rot(&mut self, rotation: glam::Quat) {
        self.rotation = rotation;
    }

    pub fn set_scale(&mut self, scale: glam::Vec3) {
        self.scale = scale;
    }

    pub fn to_homogeneous(&self) -> glam::Mat4 {
        glam::Mat4::from_translation(self.translation)
            * glam::Mat4::from_quat(self.rotation)
            * glam::Mat4::from_scale(self.scale)
    }
}

impl Default for Trs {
    fn default() -> Self {
        Self {
            translation: glam::Vec3::ZERO,
            rotation: glam::Quat::IDENTITY,
            scale: glam::Vec3::ONE,
        }
    }
}

/// ノード内に配置されたときに親からの情報を受けて変化する状態
struct NodeVars {
    fullname: String,
    world: glam::Mat4,
}

impl NodeVars {
    fn new(name: String) -> Self {
        Self {
            fullname: name,
            world: glam::Mat4::IDENTITY,
        }
    }

    fn set_fullname(&mut self, fullname: String) {
        self.fullname = fullname;
    }

    fn set_world(&mut self, world: glam::Mat4) {
        self.world = world;
    }
}

/// 表示用のノード
pub struct DrawNode {
    name: String,
    trs: Trs,
    vars: NodeVars,
}

// /// シーングラフのノードのツリー構造
// pub struct SceneGraphNode {
//     name: String,
//     trs: Trs,
//     children: Vec<SceneGraphNode>,
//     draw: Vec<DrawNode>,
//     vars: NodeVars,
// }

// impl SceneGraphNode {
//     const ROOT_NAME: &'static str = "root";

//     /// シーングラフのルートノードを作成する
//     pub fn root() -> Self {
//         Self::new(
//             Self::ROOT_NAME,
//             Trs::new(glam::Vec3::ZERO, glam::Quat::IDENTITY, glam::Vec3::ONE),
//         )
//     }

//     /// シーングラフのノードを作成する
//     pub fn new(name: &str, trs: Trs) -> Self {
//         Self {
//             name: name.to_string(),
//             trs,
//             children: vec![],
//             vars: NodeVars::new(name.to_string()),
//         }
//     }

//     // 子ノードの追加して、名前を取得
//     fn add_child_inner(&mut self, child: SceneGraphNode) -> anyhow::Result<String> {
//         if self.get_child(&child.name).is_some() {
//             return Err(anyhow::anyhow!("Child with the same name already exists"));
//         }
//         let name = child.name.clone();
//         self.children.push(child);
//         self.set_fullname(self.fullname().to_string());
//         let name = self.fine_by_name_mut(name).unwrap().fullname();
//         Ok(name.to_string())
//     }

//     /// 任意の親ノードの下に子ノードを追加する
//     pub fn add_child(
//         &mut self,
//         child: SceneGraphNode,
//         parent_name: impl Into<String>,
//     ) -> anyhow::Result<String> {
//         if self.contains(&child.name) {
//             return Err(anyhow::anyhow!("Child with the same name already exists"));
//         }
//         // 自身の子からその先もparent_nameを探して、見つかったら子ノードを追加する
//         match self.fine_by_name_mut(parent_name.into()) {
//             Some(parent) => parent.add_child_inner(child),
//             None => Err(anyhow::anyhow!("Parent node not found")),
//         }
//     }

//     fn contains(&self, name: &str) -> bool {
//         self.name == name || self.children.iter().any(|c| c.contains(name))
//     }

//     /// 子ノードの削除
//     pub fn remove_child(&mut self, name: &str) {
//         self.children.retain(|c| c.name != name);
//     }

//     /// 子ノードの取得
//     pub fn get_child(&self, name: &str) -> Option<&SceneGraphNode> {
//         self.children.iter().find(|c| c.name == name)
//     }

//     /// シーングラフ上の一意な名前を取得する
//     pub fn fullname(&self) -> &str {
//         &self.vars.fullname
//     }

//     // 子のノードに対して、名前をつける
//     pub fn set_fullname(&mut self, parent_name: String) {
//         self.vars.set_fullname(parent_name.clone());
//         for child in &mut self.children {
//             let fullname = format!("{}-{}", parent_name, child.name);
//             child.set_fullname(fullname);
//         }
//     }

//     /// 親の座標変化を受けて、ワールド座標を更新する
//     pub fn set_world(&mut self, parent_world: glam::Mat4) {
//         let world = parent_world * self.trs.to_homogeneous();
//         self.vars.set_world(world);
//         for child in &mut self.children {
//             child.set_world(world);
//         }
//     }

//     // 配下のすべてのノードにアクセスする
//     pub fn iter(&self) -> impl Iterator<Item = &SceneGraphNode> {
//         let mut stack = vec![self];
//         std::iter::from_fn(move || {
//             if let Some(node) = stack.pop() {
//                 stack.extend(node.children.iter());
//                 Some(node)
//             } else {
//                 None
//             }
//         })
//     }

//     // ノード検索実装実体
//     fn find_inner(&self, name: &[&str]) -> Option<&SceneGraphNode> {
//         let Some(node) = name.first() else {
//             return None;
//         };
//         if node == &self.name {
//             if name.len() == 1 {
//                 return Some(self);
//             }
//             for child in &self.children {
//                 if let Some(found) = child.find_inner(&name[1..]) {
//                     return Some(found);
//                 }
//             }
//             None
//         } else {
//             None
//         }
//     }

//     /// 配下から名前を指定してノードを取得する
//     pub fn find(&self, fullname: &str) -> Option<&SceneGraphNode> {
//         let keys: Vec<&str> = fullname.split('-').collect();
//         self.find_inner(&keys)
//     }

//     fn find_inner_mut(&mut self, name: &[&str]) -> Option<&mut SceneGraphNode> {
//         let Some(node) = name.first() else {
//             return None;
//         };
//         if node == &self.name {
//             if name.len() == 1 {
//                 return Some(self);
//             }
//             for child in &mut self.children {
//                 if let Some(found) = child.find_inner_mut(&name[1..]) {
//                     return Some(found);
//                 }
//             }
//             None
//         } else {
//             None
//         }
//     }

//     pub fn find_mut(&mut self, fullname: &str) -> Option<&mut SceneGraphNode> {
//         let keys: Vec<&str> = fullname.split('-').collect();
//         self.find_inner_mut(&keys)
//     }

//     /// 任意名前のノードを見つける
//     pub fn fine_by_name_mut(&mut self, name: String) -> Option<&mut SceneGraphNode> {
//         if self.name == name {
//             return Some(self);
//         }
//         for child in &mut self.children {
//             if let Some(found) = child.fine_by_name_mut(name.clone()) {
//                 return Some(found);
//             }
//         }
//         None
//     }
// }

// pub struct NodeUniform<U, B> {
//     // 座標変換情報の保持
//     buffer: UniformBuffer<U>,
//     // レンダラとのリンク情報
//     bg: B,
//     //
//     drawer: Vec<u32>,
// }

// impl<U, B> NodeUniform<U, B> {
//     pub fn new(buffer: UniformBuffer<U>, bg: B) -> Self {
//         Self { buffer, bg }
//     }

//     pub fn buffer(&self) -> &UniformBuffer<U> {
//         &self.buffer
//     }

//     pub fn bg(&self) -> &B {
//         &self.bg
//     }
// }

// /// シーングラフと描画用のユニフォームバッファを合わせたコンテキスト
// pub struct SceneContext<N> {
//     map: FxHashMap<String, N>,
//     graph: SceneGraphNode,
// }

// impl<N> SceneContext<N> {
//     pub fn add_node(
//         &mut self,
//         node: SceneGraphNode,
//         parent: impl Into<String>,
//         f: impl Fn(&SceneGraphNode) -> N,
//     ) -> anyhow::Result<()> {
//         let full_name = self.graph.add_child(node, parent)?;
//         let node = self.graph.find(&full_name).unwrap();
//         self.map.insert(full_name, f(node));
//         Ok(())
//     }

//     pub fn remove_node(&mut self, name: &str) {
//         let fullname = self.graph.find(name).unwrap().fullname();
//         if self.map.contains_key(fullname) {
//             self.map.remove(fullname);
//         }
//         self.graph.remove_child(name);
//     }

//     /// ノードの取得
//     pub fn find_mut(&mut self, name: &str) -> Option<&mut SceneGraphNode> {
//         self.graph.find_mut(name)
//     }

//     /// ワールド座標の更新
//     pub fn set_world(&mut self, world: glam::Mat4) {
//         self.graph.set_world(world);
//     }

//     pub fn keys(&self) -> impl Iterator<Item = &String> {
//         self.map.keys()
//     }

//     /// Uniform更新向けのノード取得関数
//     pub fn get_mut(&mut self, full_name: &str) -> Option<(&SceneGraphNode, &mut N)> {
//         let node = self.graph.find(full_name)?;
//         let u = self.map.get_mut(full_name)?;
//         Some((node, u))
//     }
// }

/// 一時的な表示物としてワールド座標に生成、削除を繰り返すノードを管理する
///
/// このノードの子になるものはなく、親も基本はワールド座標で更新されることがない
/// 頻繁に追加が行われ、一定条件で削除を行うことが多い
pub struct OrphanNode {}

#[cfg(test)]
mod tests {
    use fxhash::FxHashMap;
    use glam::{Vec3, Vec4, Vec4Swizzles};

    // fn print_nodes(
    //     node: &SceneGraphNode,
    //     depth: usize,
    //     fmt: &impl Fn(&SceneGraphNode, usize) -> String,
    // ) {
    //     println!("{}", fmt(node, depth));
    //     for child in &node.children {
    //         print_nodes(child, depth + 1, fmt);
    //     }
    // }

    // #[allow(unused)]
    // fn fmt_node_name(node: &SceneGraphNode, depth: usize) -> String {
    //     let indent = " ".repeat(depth * 2);
    //     format!("{}{}: {:?}", indent, node.name, node.trs.translation)
    // }

    // #[allow(unused)]
    // fn fmt_node_fullname(node: &SceneGraphNode, depth: usize) -> String {
    //     let indent = " ".repeat(depth * 2);
    //     format!("{}{}", indent, node.fullname())
    // }

    // /// シーングラフの構成と値の伝播テスト
    // #[test]
    // fn test_graph_nodes() -> anyhow::Result<()> {
    //     let mut root = SceneGraphNode::root();
    //     let mut child1 = SceneGraphNode::new("child1", Trs::with_t(Vec3::X));
    //     let child2 = SceneGraphNode::new("child2", Trs::with_t(Vec3::Y));
    //     let child3 = SceneGraphNode::new("child3", Trs::with_t(Vec3::Z));
    //     let child3_dup = SceneGraphNode::new("child3", Trs::with_t(Vec3::Z));

    //     child1.add_child_inner(child2)?;
    //     root.add_child_inner(child1)?;
    //     root.add_child_inner(child3)?;

    //     // ルートから座標更新
    //     root.set_world(glam::Mat4::IDENTITY);

    //     // 同じ名前のノードは追加できない
    //     assert!(root.add_child_inner(child3_dup).is_err());

    //     // ツリー構造を表示する
    //     print_nodes(&root, 0, &fmt_node_fullname);

    //     // それぞれのノードにアクセスする
    //     assert!(root.iter().count() == 4);

    //     // ノードフルネームで特定ノードにアクセスする
    //     let expect = [
    //         ("root", glam::Vec3::ZERO),
    //         ("root-child1", glam::Vec3::X),
    //         ("root-child1-child2", glam::Vec3::X + glam::Vec3::Y),
    //         ("root-child3", glam::Vec3::Z),
    //     ];
    //     for (i, (name, pos)) in expect.iter().enumerate() {
    //         let node = root
    //             .find(name)
    //             .unwrap_or_else(|| panic!("Node not found: [{i}] {}", name));
    //         assert_eq!(node.fullname(), *name);
    //         assert_eq!(node.vars.world, glam::Mat4::from_translation(*pos));
    //     }

    //     Ok(())
    // }

    // // nodeと親の名前のみで追加する
    // #[cfg(test)]
    // fn test_build_node_by_list() -> anyhow::Result<()> {
    //     let mut root = SceneGraphNode::root();
    //     let child1 = SceneGraphNode::new("child1", Trs::with_t(Vec3::X));
    //     let child2 = SceneGraphNode::new("child2", Trs::with_t(Vec3::Y));
    //     let child3 = SceneGraphNode::new("child3", Trs::with_t(Vec3::Z));

    //     root.add_child(child1, "root".to_string())?;
    //     root.add_child(child2, "child1".to_string())?;
    //     root.add_child(child3, "root".to_string())?;

    //     // ルートから座標更新
    //     root.set_world(glam::Mat4::IDENTITY);

    //     // ノードフルネームで特定ノードにアクセスする
    //     let expect = [
    //         ("root", glam::Vec3::ZERO),
    //         ("root-child1", glam::Vec3::X),
    //         ("root-child1-child2", glam::Vec3::X + glam::Vec3::Y),
    //         ("root-child3", glam::Vec3::Z),
    //     ];
    //     for (i, (name, pos)) in expect.iter().enumerate() {
    //         let node = root
    //             .find(name)
    //             .unwrap_or_else(|| panic!("Node not found: [{i}] {}", name));
    //         assert_eq!(node.fullname(), *name);
    //         assert_eq!(node.vars.world, glam::Mat4::from_translation(*pos));
    //     }

    //     Ok(())
    // }

    /// ルートとなるノードでシーンに対して一つのみとなる
    struct SceneNode<M> {
        model: ModelNodes<M>,
    }

    impl<M> Default for SceneNode<M>
    where
        M: Default,
    {
        fn default() -> Self {
            Self {
                model: ModelNodes::new(),
            }
        }
    }

    // モデルノードが実装するべき関数
    trait ModelNodeImpl {
        // ノードの名前
        fn name(&self) -> &str;
        // 子ノード
        fn children(&self) -> &[u64];
        fn add_child(&mut self, id: u64);
        fn remove_child(&mut self, name: u64);
        // 親ノード
        fn set_parent(&mut self, id: Option<u64>);
        fn parent(&self) -> Option<u64>;

        // 座標更新
        fn world(&self) -> glam::Mat4;
        fn update_world(&mut self, world: glam::Mat4) -> glam::Mat4;
    }

    struct ModelNodes<M> {
        map: FxHashMap<u64, M>,
        names: FxHashMap<String, u64>,
        counter: u64,
    }

    impl<M> ModelNodes<M> {
        pub fn new() -> Self {
            Self {
                map: FxHashMap::default(),
                names: FxHashMap::default(),
                counter: 0,
            }
        }

        pub fn get_node(&self, name: &str) -> Option<&M> {
            self.names.get(name).map(|v| self.map.get(v))?
        }

        fn next_id(&mut self) -> u64 {
            let id = self.counter;
            self.counter += 1;
            id
        }
    }

    impl<M> ModelNodes<M>
    where
        M: ModelNodeImpl,
    {
        pub fn add_node(&mut self, parent: Option<&str>, mut node: M) -> anyhow::Result<()> {
            if self.names.contains_key(node.name()) {
                return Err(anyhow::anyhow!(
                    "node name [{}] is already used",
                    node.name()
                ));
            }

            let (world, id) = if let Some(parent) = parent {
                let Some(parent_id) = self.names.get(parent) else {
                    return Err(anyhow::anyhow!("not found parend node [{parent}]"));
                };
                let parent_id = *parent_id;
                let id = self.next_id();
                let parent_world = {
                    let parent = self.map.get_mut(&parent_id).unwrap();
                    parent.add_child(id);
                    node.set_parent(Some(parent_id));
                    parent.world()
                };
                self.names.insert(node.name().to_string(), id);
                self.map.insert(id, node);

                (parent_world, id)
            } else {
                let id = self.next_id();
                node.set_parent(None);
                self.names.insert(node.name().to_string(), id);
                self.map.insert(id, node);
                (glam::Mat4::IDENTITY, id)
            };
            self.update_world_inner(world, &[id]);
            Ok(())
        }

        pub fn remove_node(&mut self, name: &str) -> anyhow::Result<()> {
            let Some(node_id) = self.names.remove(name) else {
                return Err(anyhow::anyhow!("not found node-id {name}"));
            };
            let Some(node) = self.map.remove(&node_id) else {
                return Err(anyhow::anyhow!("not found node {name}"));
            };
            // 親のリストから外す
            if let Some(parent_id) = node.parent() {
                // 親がremove_nodeされた場合もある
                if let Some(parent) = self.map.get_mut(&parent_id) {
                    parent.remove_child(node_id);
                };
            }
            // 子を削除
            for child in node.children() {
                let cn = self.map.get(child).unwrap().name().to_string();
                self.remove_node(&cn)?;
            }
            Ok(())
        }

        /// 任意のノードのワールド座標を更新する
        pub fn update_world(&mut self, name: &str, world: glam::Mat4) -> anyhow::Result<()> {
            let Some(node_id) = self.names.get(name) else {
                return Err(anyhow::anyhow!("not found node {name}"));
            };
            let node_id = *node_id;
            self.update_world_inner(world, &[node_id]);
            Ok(())
        }

        fn update_world_inner(&mut self, world: glam::Mat4, nodes: &[u64]) {
            for node_id in nodes {
                let node = self.map.get_mut(node_id).unwrap();
                let world = node.update_world(world);
                let children = node.children().to_vec();
                self.update_world_inner(world, &children);
            }
        }

        fn len(&self) -> usize {
            self.map.len()
        }

        fn is_empty(&self) -> bool {
            self.map.is_empty()
        }

        /// ノードの表示
        pub fn print_node(node: &M, depth: usize) {
            let indent = " ".repeat(depth * 2);
            println!("{}{}: {:?}", indent, node.name(), node.children());
        }

        fn traverse_inner(&self, node_id: u64, depth: usize, fmt: &impl Fn(&M, usize)) {
            let node = self.map.get(&node_id).unwrap();
            fmt(node, depth);
            for child in node.children() {
                self.traverse_inner(*child, depth + 1, fmt);
            }
        }

        /// ノードの表示
        pub fn traverse(&self, name: Option<&str>, fmt: &impl Fn(&M, usize)) {
            if let Some(name) = name {
                let Some(node_id) = self.names.get(name) else {
                    println!("not found node {name}");
                    return;
                };
                let node_id = *node_id;
                self.traverse_inner(node_id, 0, fmt);
            } else {
                // parentを持たないリストから、全てのノードを表示する
                let roots = self
                    .map
                    .iter()
                    .filter(|(_, node)| node.parent().is_none())
                    .map(|(id, _)| id)
                    .collect::<Vec<_>>();
                for id in roots {
                    self.traverse_inner(*id, 0, fmt);
                }
            }
        }
    }

    #[derive(Default)]
    struct DummyModel {
        name: String,
        parent: Option<u64>,
        children: Vec<u64>,
        local: glam::Mat4,
        world: glam::Mat4,
    }

    impl DummyModel {
        pub fn new(name: impl Into<String>) -> Self {
            Self {
                name: name.into(),
                parent: None,
                children: vec![],
                local: glam::Mat4::IDENTITY,
                world: glam::Mat4::IDENTITY,
            }
        }

        pub fn with_local(name: impl Into<String>, local: glam::Mat4) -> Self {
            Self {
                name: name.into(),
                parent: None,
                children: vec![],
                local,
                world: glam::Mat4::IDENTITY,
            }
        }
    }

    impl ModelNodeImpl for DummyModel {
        fn name(&self) -> &str {
            &self.name
        }

        fn children(&self) -> &[u64] {
            &self.children
        }

        fn add_child(&mut self, id: u64) {
            self.children.push(id);
        }

        fn remove_child(&mut self, id: u64) {
            self.children.retain(|&c| c != id);
        }

        fn set_parent(&mut self, id: Option<u64>) {
            self.parent = id;
        }

        fn parent(&self) -> Option<u64> {
            self.parent
        }

        fn world(&self) -> glam::Mat4 {
            self.world
        }

        fn update_world(&mut self, world: glam::Mat4) -> glam::Mat4 {
            self.world = world * self.local;
            self.world
        }
    }

    // シーンノードはすべてのオブジェクトを含む
    // カメラと物体のどちらも置けるのが望ましい
    // 名前で高速にアクセスができ、値の更新が容易でなければならない
    #[test]
    fn test_scene_node() -> anyhow::Result<()> {
        let mut scene = SceneNode::default();
        let names = [
            (None, "r1"),
            (Some("r1"), "c1"),
            (Some("c1"), "c2"),
            (Some("c1"), "c3"),
            (Some("c3"), "c4"),
            (Some("c4"), "c5"),
            (Some("c5"), "c6"),
            (None, "r2"),
            (None, "r3"),
        ];
        for (parent, name) in names {
            let node = DummyModel::new(name);
            scene.model.add_node(parent, node)?;
        }

        for (_, name) in names {
            scene.model.get_node(name).unwrap();
        }

        scene.model.traverse(None, &ModelNodes::print_node);
        assert_eq!(scene.model.len(), names.len());

        scene.model.remove_node("c1")?;
        scene.model.traverse(None, &ModelNodes::print_node);
        assert_eq!(scene.model.len(), 3);

        Ok(())
    }

    /// ワールド座標の更新
    #[test]
    fn test_scene_node_update_world() -> anyhow::Result<()> {
        let mut scene = SceneNode::default();
        let names = [
            (None, "r1", Vec3::X, Vec3::X),
            (Some("r1"), "c1", Vec3::Z, Vec3::X + Vec3::Z),
            (Some("c1"), "c2", Vec3::Y, Vec3::X + Vec3::Z + Vec3::Y),
            (Some("c1"), "c3", Vec3::X, Vec3::X + Vec3::Z + Vec3::X),
            (Some("c3"), "c4", Vec3::ZERO, Vec3::X + Vec3::Z + Vec3::X),
            (
                Some("c4"),
                "c5",
                Vec3::NEG_Y,
                Vec3::X + Vec3::Z + Vec3::X + Vec3::NEG_Y,
            ),
            (
                Some("c5"),
                "c6",
                Vec3::NEG_Z,
                Vec3::X + Vec3::X + Vec3::NEG_Y,
            ),
            (None, "r2", Vec3::Y, Vec3::Y),
            (None, "r3", Vec3::Z, Vec3::Z),
        ];
        for (parent, name, mat, _) in names {
            let node = DummyModel::with_local(name, glam::Mat4::from_translation(mat));
            scene.model.add_node(parent, node)?;
        }

        for (_, name, _, expect) in names {
            let node = scene.model.get_node(name).unwrap();
            let pos = node.world * glam::Vec4::new(0.0, 0.0, 0.0, 1.0);
            assert_eq!(pos.xyz(), expect, "name: {name}");
        }

        Ok(())
    }
}
