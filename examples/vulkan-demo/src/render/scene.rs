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

use core::str;

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

/// シーングラフのノードのツリー構造
pub struct SceneGraphNode {
    name: String,
    trs: Trs,
    children: Vec<SceneGraphNode>,
    vars: NodeVars,
}

impl SceneGraphNode {
    const ROOT_NAME: &'static str = "root";

    /// シーングラフのルートノードを作成する
    pub fn root() -> Self {
        Self::new(
            Self::ROOT_NAME,
            Trs::new(glam::Vec3::ZERO, glam::Quat::IDENTITY, glam::Vec3::ONE),
        )
    }

    /// シーングラフのノードを作成する
    pub fn new(name: &str, trs: Trs) -> Self {
        Self {
            name: name.to_string(),
            trs,
            children: vec![],
            vars: NodeVars::new(name.to_string()),
        }
    }

    /// 子ノードの追加
    pub fn add_child(&mut self, child: SceneGraphNode) -> anyhow::Result<()> {
        if self.get_child(&child.name).is_some() {
            return Err(anyhow::anyhow!("Child with the same name already exists"));
        }
        self.children.push(child);
        self.set_fullname(self.fullname().to_string());
        Ok(())
    }

    /// 子ノードの削除
    pub fn remove_child(&mut self, name: &str) {
        self.children.retain(|c| c.name != name);
    }

    /// 子ノードの取得
    pub fn get_child(&self, name: &str) -> Option<&SceneGraphNode> {
        self.children.iter().find(|c| c.name == name)
    }

    /// シーングラフ上の一意な名前を取得する
    pub fn fullname(&self) -> &str {
        &self.vars.fullname
    }

    // 子のノードに対して、名前をつける
    pub fn set_fullname(&mut self, parent_name: String) {
        self.vars.set_fullname(parent_name.clone());
        for child in &mut self.children {
            let fullname = format!("{}-{}", parent_name, child.name);
            child.set_fullname(fullname);
        }
    }

    /// 親の座標変化を受けて、ワールド座標を更新する
    pub fn set_world(&mut self, parent_world: glam::Mat4) {
        let world = parent_world * self.trs.to_homogeneous();
        self.vars.set_world(world);
        for child in &mut self.children {
            child.set_world(world);
        }
    }

    // 配下のすべてのノードにアクセスする
    pub fn iter(&self) -> impl Iterator<Item = &SceneGraphNode> {
        let mut stack = vec![self];
        std::iter::from_fn(move || {
            if let Some(node) = stack.pop() {
                stack.extend(node.children.iter());
                Some(node)
            } else {
                None
            }
        })
    }

    // ノード検索実装実体
    fn find_inner(&self, name: &[&str]) -> Option<&SceneGraphNode> {
        let Some(node) = name.first() else {
            return None;
        };
        if node == &self.name {
            if name.len() == 1 {
                return Some(self);
            }
            for child in &self.children {
                if let Some(found) = child.find_inner(&name[1..]) {
                    return Some(found);
                }
            }
            None
        } else {
            None
        }
    }

    /// 配下から名前を指定してノードを取得する
    pub fn find(&self, name: &str) -> Option<&SceneGraphNode> {
        let keys: Vec<&str> = name.split('-').collect();
        self.find_inner(&keys)
    }
}

#[cfg(test)]
mod tests {
    use glam::Vec3;

    use super::*;

    fn print_nodes(
        node: &SceneGraphNode,
        depth: usize,
        fmt: &impl Fn(&SceneGraphNode, usize) -> String,
    ) {
        println!("{}", fmt(node, depth));
        for child in &node.children {
            print_nodes(child, depth + 1, fmt);
        }
    }

    #[allow(unused)]
    fn fmt_node_name(node: &SceneGraphNode, depth: usize) -> String {
        let indent = " ".repeat(depth * 2);
        format!("{}{}: {:?}", indent, node.name, node.trs.translation)
    }

    #[allow(unused)]
    fn fmt_node_fullname(node: &SceneGraphNode, depth: usize) -> String {
        let indent = " ".repeat(depth * 2);
        format!("{}{}", indent, node.fullname())
    }

    /// シーングラフの構成と値の伝播テスト
    #[test]
    fn test_graph_nodes() -> anyhow::Result<()> {
        let mut root = SceneGraphNode::root();
        let mut child1 = SceneGraphNode::new("child1", Trs::with_t(Vec3::X));
        let child2 = SceneGraphNode::new("child2", Trs::with_t(Vec3::Y));
        let child3 = SceneGraphNode::new("child3", Trs::with_t(Vec3::Z));
        let child3_dup = SceneGraphNode::new("child3", Trs::with_t(Vec3::Z));

        child1.add_child(child2)?;
        root.add_child(child1)?;
        root.add_child(child3)?;

        // ルートから座標更新
        root.set_world(glam::Mat4::IDENTITY);

        // 同じ名前のノードは追加できない
        assert!(root.add_child(child3_dup).is_err());

        // ツリー構造を表示する
        print_nodes(&root, 0, &fmt_node_fullname);

        // それぞれのノードにアクセスする
        assert!(root.iter().count() == 4);

        // ノードフルネームで特定ノードにアクセスする
        let expect = [
            ("root", glam::Vec3::ZERO),
            ("root-child1", glam::Vec3::X),
            ("root-child1-child2", glam::Vec3::X + glam::Vec3::Y),
            ("root-child3", glam::Vec3::Z),
        ];
        for (i, (name, pos)) in expect.iter().enumerate() {
            let node = root
                .find(name)
                .unwrap_or_else(|| panic!("Node not found: [{i}] {}", name));
            assert_eq!(node.fullname(), *name);
            assert_eq!(node.vars.world, glam::Mat4::from_translation(*pos));
        }

        Ok(())
    }
}
