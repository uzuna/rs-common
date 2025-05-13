/// 型制約に関するトレイト

/// パイプライン毎に設定可能な型を制約を表現する
pub trait PipelineConstraint {
    /// カメラの型は[crate::types::uniform::Camera]で固定
    /// パイプラインごとにバインドグループの型が決まっている
    /// TODO: これは `wgsl_to_wgpu` の制約なので`wgpu::BindGroup`にするのが良いかも
    type CameraBg;
    /// モデルの型は[crate::types::uniform::Model]で固定
    /// ほかはカメラと同じ
    type ModelBg;
    // マテリアルはシェーダーで変化する
    type Material;
    type MaterialBg;
    // 頂点データタイプ
    type Vertex;

    /// シェーダーインスタンスを作る
    fn new_pipeline(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        topology: wgpu::PrimitiveTopology,
        blend: crate::prelude::Blend,
    ) -> Self;
    /// パイプラインを取得する
    fn pipeline(&self) -> &wgpu::RenderPipeline;
    /// カメラのバインドグループを作成する
    fn camera_bg(device: &wgpu::Device, buffer: &wgpu::Buffer) -> Self::CameraBg;
    /// モデルのバインドグループを作成する
    fn model_bg(device: &wgpu::Device, buffer: &wgpu::Buffer) -> Self::ModelBg;
    /// マテリアルのバインドグループを作成する
    fn material_bg(device: &wgpu::Device, buffer: &wgpu::Buffer) -> Self::MaterialBg;

    /// シェーダーごとに決まっている頂点データスロット
    /// インスタンス入力がないので基本的に0
    fn vertex_slot() -> u32 {
        0
    }
    /// デフォルトのマテリアルを取得する
    fn default_material() -> Self::Material;
}

/// [PipelineConstraint]のバインドグループが実装していることを期待するメソッド
pub trait BindGroupImpl {
    fn set(&self, pass: &mut wgpu::RenderPass<'_>);
}
