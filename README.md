# Knight Age Online (KnightOnline_402) — Docker build cho Railway

Image tối giản để treo game J2ME `KnightOnline_402.jar` trong MicroEmulator 2.0.4,
xem/điều khiển qua noVNC trong browser. Mặc định chạy **2 tab** (`acc1`, `acc2`)
với RMS + config tách riêng, vừa hạn mức Railway 2 vCPU / 1 GiB.

## Deploy Railway

1. Push repo này (kèm `vendor/`) lên GitHub.
2. Railway → New Project → Deploy from GitHub repo. Nó tự nhận `Dockerfile`
   (đã khai trong `railway.json`, builder `DOCKERFILE`).
3. Settings → Networking → Generate Domain. Railway inject `PORT`, entrypoint
   bind websockify vào đúng port đó.
4. Mở `https://<domain>/vnc.html?autoconnect=1&resize=scale`.

noVNC không đặt password (theo yêu cầu: dùng để theo dõi VPS). Ai có URL là vào
điều khiển được — đừng share domain.

## Biến môi trường

| Biến | Mặc định | Ý nghĩa |
|---|---|---|
| `ACCOUNTS` | `acc1 acc2` | Danh sách tab, cách nhau bằng space. Thêm tab = thêm tên. |
| `HEAP_MAX` | `320m` | `-Xmx` mỗi JVM |
| `MIN_HEAP_FREE` / `MAX_HEAP_FREE` | `10` / `25` | `-XX:MinHeapFreeRatio` / `MaxHeapFreeRatio`. Cho JVM **nhả page về OS** sau full GC. |
| `TRIM_INTERVAL` | `900` | Chu kỳ trim RAM (giây). `0` = tắt. |
| `TRIM_RSS_KB` | `180000` | Chỉ trim tab nào RSS vượt ngưỡng này. |
| `VNC_GEOMETRY` | `800x600` | Kích thước desktop ảo |
| `VNC_DEPTH` | `16` | Bit depth (16 nhẹ hơn 24, game chỉ vẽ sprite) |
| `DEVICE_WIDTH` / `DEVICE_HEIGHT` | `360` / `480` | `--resizableDevice` |
| `MAX_LOG_BYTES` | `5242880` | Log mỗi tab vượt ngưỡng thì truncate |

Muốn 3 tab: `ACCOUNTS=acc1 acc2 acc3`, `HEAP_MAX=224m`, `VNC_GEOMETRY=1152x600`.

## Trim RAM định kỳ

`trim_ram()` trong entrypoint chạy mỗi `TRIM_INTERVAL` giây: với mỗi tab có RSS
vượt `TRIM_RSS_KB`, gọi `jattach <pid> jcmd GC.run` → full GC. Kết hợp
`MaxHeapFreeRatio=25`, SerialGC **uncommit** phần heap không còn dùng nên RSS tụt
thật, chứ không chỉ tụt `used` trong heap.

Đo trên chính image này (process 256 MB heap thả rác):

| Trường hợp | RSS sau khi thả rác + 3× `GC.run` |
|---|---|
| Không có `HeapFreeRatio` | 281 MB → **281 MB** (heap `used` tụt, RSS đứng) |
| Có `MinHeapFreeRatio=10 MaxHeapFreeRatio=25` | 254 MB → **194 MB** |

Đó là lý do phải có cặp flag đó — gọi GC một mình vô dụng.

Lúc chỉ đứng ở menu, trim log ghi `119348kB -> 119500kB`: không giảm, vì tab chưa
sinh rác — đúng như mong đợi. Trim chỉ có tác dụng khi treo lâu và heap đã phình.
Ngưỡng `180000` kB nghĩa là tab bình thường (~120 MB) không bị pause vô ích.

## Số đo thực tế

`docker run --cpus=2 --memory=1g --memory-swap=1g`, 2 tab đã vào menu game
(`Chơi Mới / Có Tài Khoản / Máy chủ: Thiên Hà`), 1 client noVNC đang xem:

| Hạng mục | Giá trị |
|---|---|
| Image size | **533 MB** (140 MB layer nén) |
| RAM container, 2 tab | **185–190 MiB / 1 GiB** (~18%) |
| RSS mỗi JVM | ~112–122 MB |
| Xvnc / websockify / openbox | 23 / 21 / 19 MB |
| CPU, 2 tab ở menu + 1 viewer | ~11–14% của 2 vCPU |

Đường đi của tối ưu, đo từng bước:

| Bước | Image | RAM 2 tab |
|---|---|---|
| Bỏ desktop/firefox/systemd, base JRE | 969 MB | — |
| Purge mesa+llvm+numpy chain | 631 MB | 196 MiB |
| jlink runtime 54 MB, `-Xmx320m`, trim | 519 MB | 241 MiB |
| + CDS archive (`-Xshare:dump`) | **533 MB** | **185 MiB** |

jlink một mình làm RAM *tăng* (241 MiB) vì runtime tự build không có CDS archive
nên mỗi JVM phải tự nạp và giữ metadata class riêng. Thêm 14 MB archive vào image
để 2 JVM `mmap` chung → RAM tụt xuống thấp nhất trong tất cả biến thể. Đổi 14 MB
đĩa lấy 56 MiB RAM.

## Đã kiểm chứng

- 2 tab MicroEmulator vào tới menu game, click được, gõ chữ được (`xdotool type`).
- `PORT=8080` → websockify bind đúng, `vnc.html` trả 200.
- `ACCOUNTS="a1 a2 a3"` → 3 JVM, 3 cửa sổ, 3 account dir riêng.
- `kill -9` một tab → supervisor start lại tab đó, số cửa sổ trở về 2.
- `docker stop` → exit code 0 trong ~7 s, không bị SIGKILL sau timeout.
- RMS tách biệt: có cả `accounts/acc1/home/.microemulator/config2.xml` và `acc2/...`.
- Từ container resolve + mở được TCP `hs1.teamobi.com:19129` và
  `hsglobal.teamobi.com:19129` (host lấy từ `dx.class` trong game jar).
- `sha256sum -c` cả 3 artifact ngay trong build.

## Đã bỏ những gì so với image gốc

Image gốc là "Ubuntu desktop qua noVNC", không có Java nên không chạy được MicroEmulator.
Bỏ: `xfce4`, `xfce4-goodies`, `firefox` + PPA mozillateam, `snapd`, `systemd`, `init`,
`xterm`, `vim`, `git`, `net-tools`, `xubuntu-icon-theme`, `x11-apps`, `dbus-x11`,
`software-properties-common`, `sudo`.

Giữ: `tigervnc-standalone-server` (Xvnc), `novnc` + `websockify`, `openbox` (focus +
khung cửa sổ di chuyển được), `xdotool` (tile cửa sổ lúc boot), `fonts-dejavu-core`
+ `fontconfig` + `libfreetype6`, `libxext6`/`libxi6`/`libxrender1`/`libxtst6`.

Không cài audio stack: quét toàn bộ 187 class của game jar chỉ thấy
`javax.microedition.{lcdui,io,rms,midlet}`, không có `javax.microedition.media`
→ không có đường code phát tiếng.

Purge sau khi apt cài (đã kiểm chứng Xvnc + openbox + noVNC + emulator vẫn chạy):
`libgl1-mesa-dri` + `libllvm15` (~146 MB, chỉ là `Recommends` của tigervnc cho GLX
— software framebuffer không dùng), `python3-numpy`/`python3-babel`/
`python-babel-localedata`/`python3-netaddr`/`ieee-data`/`liblapack3`/`libblas3`/
`libgfortran5`/`libquadmath0` (websockify kéo theo, không dùng),
`perl-modules-5.34`/`libperl5.34`.

Java không dùng image JRE nữa mà `jlink` runtime chỉ gồm 10 module cần thiết:
**136 MB → 54 MB**.

### Đã cân nhắc rồi bỏ

- **Bỏ luôn openbox** (tiết kiệm thêm 24 MB đĩa + 19 MB RAM): chạy được, click và
  gõ chữ vẫn vào MIDlet vì Xvnc tự cấp PointerRoot focus. Nhưng không có title bar
  → không kéo/xếp lại cửa sổ qua noVNC, và không thể focus tab kia bằng click. Giữ
  openbox vì đây là tool để anh theo dõi bằng mắt.
- **flwm/matchbox** (WM ít package hơn): không có trong repo Ubuntu 22.04 jammy.

## Cách 2 tab không đè nhau

`org/microemu/app/Config.class` dựng đường config từ `user.home` + `/.microemulator/`
+ `config2.xml`, và `--rms file` ghi RMS trong cùng gốc đó. Nên mỗi tab chạy với
`-Duser.home=/opt/knight/accounts/<acc>/home` và `-Djava.io.tmpdir=.../tmp` là đủ
tách hoàn toàn.

## JVM flags

```
-Xms8m -Xmx320m -Xss512k -XX:+UseSerialGC
-XX:ReservedCodeCacheSize=32m -XX:MaxMetaspaceSize=96m -XX:-UsePerfData
-XX:MinHeapFreeRatio=10 -XX:MaxHeapFreeRatio=25
```

Giữ đúng flags gốc của anh, chỉ đổi: `-Xmx` 192m → 320m, `MaxMetaspaceSize` 64m →
96m (đo thực tế metaspace dùng 15 MB, 64m vẫn đủ nhưng 96m là trần an toàn khi
game load nhiều class hơn lúc vào map), thêm `-XX:-UsePerfData` (bỏ
`/tmp/hsperfdata`) và cặp `HeapFreeRatio`. `MALLOC_ARENA_MAX=2` ở env, chặn glibc
phình arena theo thread trên máy nhiều core.

`-Xms8m` giữ nguyên: heap lớn dần theo nhu cầu, không chiếm trước.

## Supervisor

`bin/entrypoint.sh` là PID 1: Xvnc → openbox → websockify → N tab, rồi vòng lặp
10 s. Tab java chết thì start lại tab đó. Xvnc hoặc websockify chết thì thoát
non-zero để Railway restart cả container (`restartPolicyType: ALWAYS`).

## Build/chạy local

```bash
docker build -t knight-novnc .
docker run -d --name knight --cpus=2 --memory=1g --memory-swap=1g -p 6080:6080 knight-novnc
# http://localhost:6080/vnc.html?autoconnect=1&resize=scale
```
