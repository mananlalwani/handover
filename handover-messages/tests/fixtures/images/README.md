# Image fixtures

These are original synthetic decoder inputs, generated locally with ImageMagick. The 80×40 source has red, green, and blue vertical bands. JPEG, WebP, HEIC, AVIF, and JXL use lossy encoding; the others preserve the source colors. ICO uses a centered 40×40 crop, encoded at 32×32 to fit the icon format.

All images are still frames. The GIF contains one frame, so it does not test animation handling.

Recreate them with ImageMagick 7 and the matching format delegates installed:

```sh
magick -size 80x40 xc:'#e53935' -fill '#43a047' -draw 'rectangle 27,0 53,39' \
  -fill '#1e88e5' -draw 'rectangle 54,0 79,39' /tmp/source.png
magick /tmp/source.png -quality 82 sample.jpg
cp /tmp/source.png sample.png
magick /tmp/source.png sample.gif
magick /tmp/source.png -quality 80 sample.webp
magick /tmp/source.png BMP3:sample.bmp
magick /tmp/source.png -type TrueColor -depth 8 sample.tiff
magick /tmp/source.png -gravity center -crop 40x40+0+0 +repage \
  -define icon:auto-resize=32 sample.ico
magick /tmp/source.png -quality 55 sample.heic
magick /tmp/source.png -quality 55 sample.avif
magick /tmp/source.png -quality 55 sample.jxl
```
