
# get native target-features

実行中のCPUで有効なfeatureを確認する

```sh
rustc --print target-features > target-features.txt
awk -F ' - ' '{gsub(/ /, "", $1); print "\"" $1 "\","}' target-features.txt
```

## list target-features and cpus

特定のtripleで指定可能なfeature,cpuのリストを表示する

```sh
rustc --target=${TRIPLE} --print target-features
rustc --target=${TRIPLE} --print target-cpus
```
