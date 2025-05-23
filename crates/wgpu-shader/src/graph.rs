//! モデルグラフ実装
//!
//! 3次元空間のレンダリングでは、最終的にレンダリング空間の一意の位置を示すワールド座標上の位置と
//! それを表示する2次元平面に落とす変換行列を得る必要があります。
//! 具体的なレンダリングの流れは[Vulkan Tutorial: Graphics Pipeline](https://vulkan-tutorial.com/Drawing_a_triangle/Graphics_pipeline_basics/Introduction)を参照してください。
//!
//! シーングラフは人間が三次元空間にモノを配置するときの考え方と、レンダリングパイプラインに必要な情報を結びつけるためのデータ構造です。
//! 人間のデフォルトの認識であるローカル座標(自分の視点や、特定のものからの相対距離)で位置の定義を行い、
//! モノの親子関係を割り振ることでワールド座標への変換を可能にします。
//!
//! シーングラフは、3Dを扱うアプリケーションにおいて一般的な概念です。
//! 具体的な例として[Unityのシーン](https://docs.unity3d.com/ja/2022.3/Manual/CreatingScenes.html)やBlenderの[シーン](https://docs.blender.org/manual/ja/latest/scene_layout/index.html)を参照してください。

use fxhash::FxHashMap;
use glam::Quat;

pub struct TrsUpdate {
    pub translation: Option<glam::Vec3>,
    pub rotation: Option<glam::Quat>,
    pub scale: Option<glam::Vec3>,
}

/// 移動-回転-拡大の等長写像変換の定義型です。[ModelNode]のローカル座標として利用します
///
/// TRSとしているのはそれが人間から見て匝瑳市やすさのためで、
/// 最終出力としてはアフィン変換と投資投影に使える[glam::Mat4]を得ることがこの型の目的です
#[derive(Debug, Clone)]
pub struct Trs {
    pub translation: glam::Vec3,
    pub rotation: glam::Quat,
    pub scale: glam::Vec3,
}

impl Trs {
    /// TRSを指定して生成
    pub fn new(translation: glam::Vec3, rotation: glam::Quat, scale: glam::Vec3) -> Self {
        Self {
            translation,
            rotation,
            scale,
        }
    }

    /// 移動-回転を指定して生成
    pub fn with_tr(translation: glam::Vec3, rotation: glam::Quat) -> Self {
        Self::new(translation, rotation, glam::Vec3::ONE)
    }

    /// 移動を指定して生成
    pub fn with_t(translation: glam::Vec3) -> Self {
        Self::new(translation, glam::Quat::IDENTITY, glam::Vec3::ONE)
    }

    /// 拡大を指定して生成
    pub fn with_s(scale: f32) -> Self {
        Self::new(
            glam::Vec3::ZERO,
            glam::Quat::IDENTITY,
            glam::Vec3::splat(scale),
        )
    }

    /// 移動を上書き
    pub fn set_translation(&mut self, translation: glam::Vec3) {
        self.translation = translation;
    }

    /// X軸の回転を上書き
    pub fn set_rot_x(&mut self, angle: f32) {
        self.rotation = glam::Quat::from_rotation_x(angle);
    }

    /// Y軸の回転を上書き
    pub fn set_rot_y(&mut self, angle: f32) {
        self.rotation = glam::Quat::from_rotation_y(angle);
    }

    /// Z軸の回転を上書き
    pub fn set_rot_z(&mut self, angle: f32) {
        self.rotation = glam::Quat::from_rotation_z(angle);
    }

    /// クオータニオンで上書き
    pub fn set_rot(&mut self, rotation: glam::Quat) {
        self.rotation = rotation;
    }

    /// スケールを上書き
    pub fn set_scale(&mut self, scale: glam::Vec3) {
        self.scale = scale;
    }

    /// WebGPU Uniform向けの行列を取得
    pub fn to_homogeneous(&self) -> glam::Mat4 {
        glam::Mat4::from_translation(self.translation)
            * glam::Mat4::from_quat(self.rotation)
            * glam::Mat4::from_scale(self.scale)
    }

    /// 行列からTRSを復元する。
    ///
    /// # 注意
    /// スケールが1.0ではない場合にクオータニオンは正しく復元できない
    pub fn from_homogeneous(mat: glam::Mat4) -> Self {
        let translation = mat.w_axis.truncate();
        let rotation = glam::Quat::from_mat4(&mat);
        let scale = glam::Vec3::new(
            mat.x_axis.length(),
            mat.y_axis.length(),
            mat.z_axis.length(),
        );
        Self {
            translation,
            rotation,
            scale,
        }
    }

    pub fn transform(&self, mat: &glam::Mat4) -> Self {
        let translation = mat.transform_vector3(self.translation);
        let q = mat.transform_vector3(self.rotation.xyz());
        let scale = mat.transform_vector3(self.scale).abs();
        let rotation = Quat::from_xyzw(q.x, q.y, q.z, self.rotation.w);
        Self {
            translation,
            rotation,
            scale,
        }
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

/// [ModelGraph]が持てるノードの実装を定義するトレイトです
///
/// [ModelNode]は実装しているので、これを気にする必要がありません
pub trait ModelNodeImpl {
    /// ノードの名前を返す。シーン全体で一意でなければならない
    fn name(&self) -> &str;
    // 子ノードへのアクセス方法
    fn children(&self) -> &[u64];
    fn add_child(&mut self, id: u64);
    fn remove_child(&mut self, name: u64);

    // 親ノードへのアクセス方法
    fn set_parent(&mut self, id: Option<u64>);
    fn parent(&self) -> Option<u64>;

    // 座標更新に関する実装
    fn world(&self) -> glam::Mat4;
    fn local(&self) -> &Trs;
    fn update_world(&mut self, world: glam::Mat4) -> glam::Mat4;
}

/// [ModelGraph]がレンダリング順序を計算するのに必要なトレイトです
/// 3Dでもレンダリング順序を意識するべきシーンがあります。
/// 例えば半透明オブジェクトやドロップシャドウは通常物体の後に描画する必要があります。
/// このトレイトでレンダリング順序を守ることで期待される表示結果を得ることができます。
pub trait ModelNodeImplRenderable {
    /// レンダリング対象かどうかを返す
    fn is_renderable(&self) -> bool;
    /// レンダリング対象のノードの順序比較を行う
    fn render_ord(&self, other: &Self) -> std::cmp::Ordering;
}

/// 3Dモデル向けのグラフ構造体でノードとして[ModelNode]を持つことを想定しています
///
/// [ModelNode]はユーザーの構造体を保持して具体的な型を実装することを期待しています。
/// シーングラフとしていないのは、ここには空間に配置するノード飲みを取り扱っており、
/// シーン全体の設定などさらなる付加的情報を持たないためです。
///
/// 実装の方向としては、ツリー構造の変更頻度が低くノードプロパティの更新が頻繁に行われるケースを想定しています。
/// 外部からノードの名前を使ったアクセスが頻繁に耐えるために[Self::get_node_mut]を持っています。
/// 変更差分のみGPU弐転送できるように[ModelNode::get_updated]で更新対象を制限することができます。
pub struct ModelGraph<M> {
    // ノードの所有権を持つマップ。グラフが自動採番を行う
    map: FxHashMap<u64, M>,
    // ユーザー指定のノード名でノード弐アクセスするためのマップ
    names: FxHashMap<String, u64>,
    // ノードIDの自動採番用カウンタ
    counter: u64,
    // レンダリング対象ノードを、レンダリング順序別に並べたもの
    renderable_ord: Vec<u64>,
}

impl<M> Default for ModelGraph<M> {
    fn default() -> Self {
        Self::new()
    }
}

impl<M> ModelGraph<M> {
    /// からのグラフを作成する
    pub fn new() -> Self {
        Self {
            map: FxHashMap::default(),
            names: FxHashMap::default(),
            counter: 0,
            renderable_ord: vec![],
        }
    }

    /// ノードの参照を取得
    pub fn get_node(&self, name: &str) -> Option<&M> {
        self.names.get(name).map(|v| self.map.get(v))?
    }

    /// ノードの可変参照を取得
    pub fn get_node_mut(&mut self, name: &str) -> Option<&mut M> {
        self.names.get(name).map(|v| self.map.get_mut(v))?
    }

    /// [Self::get_node_mut]の代わり、ノードが確実にある場合にunwrapを省略する書き方
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

    /// ノードの名前を取得します
    pub fn keys(&self) -> impl Iterator<Item = &str> {
        self.names.keys().map(|s| s.as_str())
    }
}

impl<M> ModelGraph<M>
where
    M: ModelNodeImpl,
{
    /// ノードを追加する
    ///
    /// parent: 親ノードの名前。グローバル直下に追加する場合はNoneを指定します
    /// node: 追加するノード
    pub fn add_node(&mut self, parent: Option<&str>, mut node: M) -> anyhow::Result<()> {
        if self.names.contains_key(node.name()) {
            return Err(anyhow::anyhow!(
                "node name [{}] is already used",
                node.name()
            ));
        }

        // 親がある場合は探して親子関係を設定する
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
            // 親がない場合はグローバル直下に追加する
            let id = self.next_id();
            node.set_parent(None);
            self.names.insert(node.name().to_string(), id);
            self.map.insert(id, node);
            (glam::Mat4::IDENTITY, id)
        };
        // 親子関係に基づいてワールド座標を更新する
        self.update_world_inner(world, &[id], &mut |_, _| {});
        Ok(())
    }

    /// ノードの削除。子ノードも全て削除される
    ///
    /// name: ノードの名前
    pub fn remove_node(&mut self, name: &str) -> anyhow::Result<()> {
        // 対象ノードの実態と名前を削除
        let Some(node_id) = self.names.remove(name) else {
            return Err(anyhow::anyhow!("not found node-id {name}"));
        };
        let Some(node) = self.map.remove(&node_id) else {
            return Err(anyhow::anyhow!("not found node {name}"));
        };
        // ノードに親がある場合は、親から自ノードを削除
        if let Some(parent_id) = node.parent() {
            // 親がremove_nodeされた場合もある
            if let Some(parent) = self.map.get_mut(&parent_id) {
                parent.remove_child(node_id);
            };
        }
        // 子ノードを全て削除
        for child in node.children() {
            let cn = self.map.get(child).unwrap().name().to_string();
            self.remove_node(&cn)?;
        }
        Ok(())
    }

    pub fn contains_key(&self, name: &str) -> bool {
        self.names.contains_key(name)
    }

    /// 任意ノードとその子のワールド座標を更新します。local座標を更新した後はこれを呼んでください
    pub fn update_world(&mut self, name: &str) -> anyhow::Result<()> {
        // 指定されたノードの親のワールド座標を取得して再帰更新をする
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
        self.update_world_inner(world, &[node_id], &mut |_, _| {});
        Ok(())
    }

    // ワールド座標再帰更新の実装です
    fn update_world_inner(
        &mut self,
        world: glam::Mat4,
        nodes: &[u64],
        f: &mut impl FnMut(&str, glam::Mat4),
    ) {
        for node_id in nodes {
            let node = self.map.get_mut(node_id).unwrap();
            let world = node.update_world(world);
            f(node.name(), world);
            let children = node.children().to_vec();
            self.update_world_inner(world, &children, f);
        }
    }

    /// 更新結果をコールバックで受け取ります
    pub fn update_world_with_cb(
        &mut self,
        name: &str,
        f: &mut impl FnMut(&str, glam::Mat4),
    ) -> anyhow::Result<()> {
        // 指定されたノードの親のワールド座標を取得して再帰更新をする
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
        self.update_world_inner(world, &[node_id], f);
        Ok(())
    }

    /// グラフが持つノード数を取得します
    pub fn len(&self) -> usize {
        self.map.len()
    }

    /// グラフにノードがない場合にtrueを返します
    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    /// ノードのイテレータを取得します
    pub fn iter(&self) -> impl Iterator<Item = &M> {
        self.map.values()
    }

    /// ノードの可変イテレータを取得します
    pub fn iter_mut(&mut self) -> impl Iterator<Item = &mut M> {
        self.map.values_mut()
    }

    /// ツリー走査(トラバース)に渡す表示関数。ノード名と子ノードのIDを表示します
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

    /// ノードのツリーを走査します
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

    fn traverse_pair_inner(
        &self,
        node_id: u64,
        parent: Option<&M>,
        f: &mut impl FnMut(Option<&M>, &M) -> anyhow::Result<()>,
    ) -> anyhow::Result<()> {
        let node = self.map.get(&node_id).unwrap();
        f(parent, node)?;
        for child in node.children() {
            self.traverse_pair_inner(*child, Some(node), f)?;
        }
        Ok(())
    }

    /// 親子関係がわかる形でトラバース
    pub fn traverse_pair(
        &self,
        f: &mut impl FnMut(Option<&M>, &M) -> anyhow::Result<()>,
    ) -> anyhow::Result<()> {
        // parentを持たないリストから、全てのノードを表示する
        let roots = self
            .map
            .iter()
            .filter(|(_, node)| node.parent().is_none())
            .map(|(id, _)| id)
            .collect::<Vec<_>>();
        for id in roots {
            self.traverse_pair_inner(*id, None, f)?;
        }
        Ok(())
    }
}

impl<M> ModelGraph<M>
where
    M: ModelNodeImpl + ModelNodeImplRenderable,
{
    /// 現在のノードのレンダリング順序を更新します
    pub fn update_renderable_ord(&mut self) {
        let mut l = self
            .map
            .iter()
            .filter(|(_, node)| node.is_renderable())
            .collect::<Vec<_>>();
        l.sort_by(|(_, a), (_, b)| a.render_ord(b));
        self.renderable_ord = l.into_iter().map(|(id, _)| *id).collect();
    }

    /// レンダリング順にノードを取得します
    pub fn iter_ordered(&self) -> impl Iterator<Item = &M> {
        self.renderable_ord.iter().filter_map(|id| self.map.get(id))
    }
}

/// グラフノードの基本実装です。[Trs]やノード名、親子関係を持ちます
///
/// 実際に利用する情報は[ModelNode::value]に格納します
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
    // ワールド座標が更新され、それを読み出したかどうか
    world_updated: bool,
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
            world_updated: false,
            value: Default::default(),
        }
    }
}

impl<T> ModelNode<T> {
    /// ノード識別子とローカル座標を指定してノードを作成します
    ///
    /// name: ノードの名前です。シーン全体で一意なものにしてください
    /// local: ノードのローカル座標。親ノードからの相対位置
    /// value: ユーザーデータ
    pub fn new(name: impl Into<String>, local: Trs, value: T) -> Self {
        Self {
            name: name.into(),
            parent: None,
            children: vec![],
            local,
            world: glam::Mat4::IDENTITY,
            world_updated: true,
            value,
        }
    }

    /// [Self::new]の内、ローカル座標を省略して生成します
    pub fn with_value(name: impl Into<String>, value: T) -> Self {
        Self::new(name, Trs::default(), value)
    }

    /// ユーザーデータへの参照を取得します
    pub fn value(&self) -> &T {
        &self.value
    }

    /// ユーザーデータへの可変参照を取得します
    pub fn value_mut(&mut self) -> &mut T {
        &mut self.value
    }

    pub fn trs(&self) -> &Trs {
        &self.local
    }

    /// local座標への可変参照を取得します
    pub fn trs_mut(&mut self) -> &mut Trs {
        &mut self.local
    }

    /// ワールド座標が更新されたかときにtrueを返し、フラグを折ります
    /// GPU側に行列データの転送が必要か知るためのメソッドです
    pub fn get_updated(&mut self) -> bool {
        let flag = self.world_updated;
        self.world_updated = false;
        flag
    }

    /// 名前とローカル座標をそのままに、ユーザーデータを複製したノードを作成します
    pub fn duplicate_with<U>(&self, value: U) -> ModelNode<U> {
        ModelNode::new(self.name.clone(), self.local.clone(), value)
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

    fn local(&self) -> &Trs {
        &self.local
    }

    // グラフノードの親からのワールド座標を受け取り、ローカル座標を掛け算してワールド座標を更新します
    fn update_world(&mut self, world: glam::Mat4) -> glam::Mat4 {
        self.world_updated = true;
        self.world = world * self.local.to_homogeneous();
        self.world
    }
}

impl<T> ModelNodeImplRenderable for ModelNode<T>
where
    T: ModelNodeImplRenderable,
{
    fn is_renderable(&self) -> bool {
        self.value.is_renderable()
    }

    fn render_ord(&self, other: &Self) -> std::cmp::Ordering {
        self.value.render_ord(&other.value)
    }
}

#[cfg(test)]
mod tests {
    use glam::{Vec3, Vec4Swizzles as _};

    use crate::graph::ModelGraph;

    use super::{ModelNode, Trs};

    type DummyModel = ModelNode<()>;

    // ノードの追加、削除、走査のテスト
    #[test]
    fn test_model_graph() -> anyhow::Result<()> {
        let mut model = ModelGraph::new();
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
            model.add_node(parent, node)?;
        }

        for (_, name) in names {
            model.get_node(name).unwrap();
        }

        model.traverse(None, &ModelGraph::print_node);
        assert_eq!(model.len(), names.len());

        model.remove_node("c1")?;
        model.traverse(None, &ModelGraph::print_node);
        assert_eq!(model.len(), 3);

        Ok(())
    }

    /// ワールド座標の伝播を確認する
    #[test]
    fn test_model_graph_update_world() -> anyhow::Result<()> {
        let mut model = ModelGraph::new();
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
            model.add_node(parent, node)?;
        }

        for (_, name, _, expect) in names {
            let node = model.get_node(name).unwrap();
            let pos = node.world * glam::Vec4::new(0.0, 0.0, 0.0, 1.0);
            assert_eq!(pos.xyz(), expect, "name: {name}");
            assert!(node.world_updated);
        }

        model.get_must_mut("r1").trs_mut().set_translation(Vec3::Y);
        model.update_world("r1")?;

        for (_, name, _, expect) in names {
            let node = model.get_node(name).unwrap();
            let pos = node.world * glam::Vec4::new(0.0, 0.0, 0.0, 1.0);
            // r1以下は親の影響を受けるので、r1の移動分を加算する
            if name == "r2" || name == "r3" {
                assert_eq!(pos.xyz(), expect, "name: {name}");
            } else {
                assert_eq!(pos.xyz(), expect + Vec3::Y - Vec3::X, "name: {name}");
            }
            assert!(node.world_updated);
        }

        Ok(())
    }

    #[test]
    fn test_homogeneous() {
        let trs = Trs::new(
            glam::Vec3::new(1.0, 2.0, 3.0),
            glam::Quat::from_rotation_y(0.5),
            glam::Vec3::new(1.0, 1.0, 1.0),
        );
        let mat = trs.to_homogeneous();
        let trs2 = Trs::from_homogeneous(mat);
        assert_eq!(trs.translation, trs2.translation);
        assert_eq!(trs.scale, trs2.scale);
        approx::assert_abs_diff_eq!(trs.rotation.x, trs2.rotation.x, epsilon = 0.0001);
        approx::assert_abs_diff_eq!(trs.rotation.y, trs2.rotation.y, epsilon = 0.0001);
        approx::assert_abs_diff_eq!(trs.rotation.z, trs2.rotation.z, epsilon = 0.0001);
    }
}
