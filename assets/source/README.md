# 应用图标源文件

`sidera-artwork.png` 是原 `sidera.svg` 内嵌的 512×512 原画，保留标志和已有圆角边框。

`python tools/refresh_icon.py` 使用 Pillow 沿原边框生成带透明圆角、抗锯齿边缘的图标，统一导出根目录 PNG、内嵌位图的 SVG、Windows ICO 和文档独立图标。更改轮廓时修改该脚本，避免多个版本的图标偏离。

软件启动页和关于页共享编译进程序的 PNG；Linux 桌面入口使用安装包中的对应资源，Windows 托盘使用 ICO。
