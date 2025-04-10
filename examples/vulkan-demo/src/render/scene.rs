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

use fxhash::FxHashMap;

/// 各ノードのTRS操作
#[derive(Clone)]
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

    pub fn with_s(scale: f32) -> Self {
        Self::new(
            glam::Vec3::ZERO,
            glam::Quat::IDENTITY,
            glam::Vec3::splat(scale),
        )
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

/// 表示用のノード
pub struct DrawNode {
    name: String,
    trs: Trs,
}

/// ルートとなるノードでシーンに対して一つのみとなる
pub struct SceneNode<M> {
    model: ModelNodes<M>,
}

impl<M> Default for SceneNode<M> {
    fn default() -> Self {
        Self {
            model: ModelNodes::new(),
        }
    }
}

impl<M> SceneNode<M> {
    pub fn model(&self) -> &ModelNodes<M> {
        &self.model
    }

    pub fn model_mut(&mut self) -> &mut ModelNodes<M> {
        &mut self.model
    }
}

/// [ModelNodes]が期待するノードが実装するべき関数
pub trait ModelNodeImpl {
    // ノードの名前
    fn name(&self) -> &str;
    // 子ノード
    fn children(&self) -> &[u64];
    fn add_child(&mut self, id: u64);
    fn remove_child(&mut self, name: u64);
    // 親ノード
    fn set_parent(&mut self, id: Option<u64>);
    fn parent(&self) -> Option<u64>;

    // 座標更新に関する実装
    fn world(&self) -> glam::Mat4;
    fn update_world(&mut self, world: glam::Mat4) -> glam::Mat4;
}

pub trait ModelNodeImplClone {
    fn clone_object(&self, device: &wgpu::Device) -> Self;
}

/// モデルノードの管理
pub struct ModelNodes<M> {
    // ノードの保持
    map: FxHashMap<u64, M>,
    // 名前でノードにアクセスするためのマップ
    names: FxHashMap<String, u64>,
    // ノードID割当のカウンタ
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

    /// ノードの取得
    pub fn get_node(&self, name: &str) -> Option<&M> {
        self.names.get(name).map(|v| self.map.get(v))?
    }

    /// ノードの取得
    pub fn get_node_mut(&mut self, name: &str) -> Option<&mut M> {
        self.names.get(name).map(|v| self.map.get_mut(v))?
    }

    /// ノードが確実にある場合にunwrapを省略する。[Self::get_node_mut]
    pub fn get_must_mut(&mut self, name: &str) -> &mut M {
        self.get_node_mut(name)
            .unwrap_or_else(|| panic!("not found node {name}"))
    }

    // NodeのIDを取得
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
    /// ノードの追加
    ///
    /// parent: 親ノードの名前
    /// node: 追加するノード
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

    /// ノードの削除
    ///
    /// name: ノードの名前
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

    /// 特定ノードのlocalを更新した場合に、その下のノードのworldを更新する
    pub fn update_world(&mut self, name: &str) -> anyhow::Result<()> {
        let Some(node_id) = self.names.get(name) else {
            return Err(anyhow::anyhow!("not found node {name}"));
        };
        let node_id = *node_id;
        let world = {
            let node = self.map.get_mut(&node_id).unwrap();
            node.parent()
                .map(|id| match self.map.get(&id) {
                    Some(parent) => parent.world(),
                    None => glam::Mat4::IDENTITY,
                })
                .unwrap_or(glam::Mat4::IDENTITY)
        };
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

    /// ノードの数を取得
    pub fn len(&self) -> usize {
        self.map.len()
    }

    /// ノードを持っていないかどうか
    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = &M> {
        self.map.values()
    }

    pub fn iter_mut(&mut self) -> impl Iterator<Item = &mut M> {
        self.map.values_mut()
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

/// ノードの基本実装
///
/// ユーザーが持たせたい情報は[ModelNode<T>]に格納する
pub struct ModelNode<T> {
    // ノードの名前
    name: String,
    // 親ノードのID
    parent: Option<u64>,
    // 子ノードのID
    children: Vec<u64>,
    // ローカル座標
    local: Trs,
    // ノードグラフからたどって作られたワールド座標
    world: glam::Mat4,
    // world座標を更新した場合にtrueになる
    update_flag: bool,
    // ユーザー固有データ
    value: T,
}

impl<T: Default> Default for ModelNode<T> {
    fn default() -> Self {
        Self {
            name: String::new(),
            parent: None,
            children: vec![],
            local: Trs::default(),
            world: glam::Mat4::IDENTITY,
            update_flag: false,
            value: Default::default(),
        }
    }
}

impl<T> ModelNode<T> {
    pub fn new(name: impl Into<String>, local: Trs, value: T) -> Self {
        Self {
            name: name.into(),
            parent: None,
            children: vec![],
            local,
            world: glam::Mat4::IDENTITY,
            update_flag: false,
            value,
        }
    }

    pub fn with_value(name: impl Into<String>, value: T) -> Self {
        Self::new(name, Trs::default(), value)
    }

    pub fn value(&self) -> &T {
        &self.value
    }

    pub fn value_mut(&mut self) -> &mut T {
        &mut self.value
    }

    /// local座標を編集する
    pub fn trs_mut(&mut self) -> &mut Trs {
        &mut self.local
    }

    /// 更新フラグを確認して折る
    pub fn get_update(&mut self) -> bool {
        let flag = self.update_flag;
        self.update_flag = false;
        flag
    }
}

impl<T> ModelNodeImpl for ModelNode<T> {
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
        self.update_flag = true;
        self.world = world * self.local.to_homogeneous();
        self.world
    }
}

impl<T> ModelNodeImplClone for ModelNode<T>
where
    T: ModelNodeImplClone,
{
    fn clone_object(&self, device: &wgpu::Device) -> Self {
        Self {
            name: self.name.clone(),
            parent: self.parent,
            children: self.children.clone(),
            local: self.local.clone(),
            world: self.world,
            update_flag: self.update_flag,
            value: self.value.clone_object(device),
        }
    }
}

#[cfg(test)]
mod tests {
    use glam::{Vec3, Vec4Swizzles as _};

    use crate::render::scene::{ModelNodes, SceneNode};

    use super::{ModelNode, Trs};

    type DummyModel = ModelNode<()>;

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
            let node = DummyModel::with_value(name, ());
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
        for (parent, name, t, _) in names {
            let node = DummyModel::new(name, Trs::with_t(t), ());
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
