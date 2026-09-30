# Maintainer: rask <rask@example.com>
pkgname=larp-music-player
pkgver=0.1.0
pkgrel=1
pkgdesc="TUI music player: Yandex/iTunes/SoundCloud/YouTubeMusic search, playback, lyrics (LRCLIB/NetEase), MPRIS + Discord integration"
arch=('x86_64')
url="https://music.163.com/"
license=('custom')
depends=('yt-dlp' 'ffmpeg' 'alsa-lib' 'openssl' 'python' 'python-ytmusicapi')
makedepends=('cargo' 'pkg-config')
options=('!lto')
source=("$pkgname-$pkgver.tar.gz")
sha256sums=('SKIP')

prepare() {
  cd "$srcdir/$pkgname-$pkgver"
  cargo fetch --locked
}

build() {
  cd "$srcdir/$pkgname-$pkgver"
  cargo build --release --locked
}

check() {
  cd "$srcdir/$pkgname-$pkgver"
  cargo test --release --locked
}

package() {
  install -Dm755 "$srcdir/$pkgname-$pkgver/target/release/$pkgname" \
    "$pkgdir/usr/bin/$pkgname"
  install -Dm644 "$srcdir/$pkgname-$pkgver/ytmeta.py" \
    "$pkgdir/usr/share/$pkgname/ytmeta.py"
}
