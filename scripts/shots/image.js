// Picture helpers for scripts/shot, in JavaScript for Automation (`osascript -l JavaScript`):
// AppKit from the system, no other program needed.
//   stats <png>                          {"w":..,"h":..,"colors":..,"clear":..}: on a 64x40 grid,
//                                        the colours, and the samples see-through (not drawn)
//   pair <before.png|-> <after.png> <out.png> <left label> <right label>
//                                        the two side by side, labelled; `-`: after alone
ObjC.import('AppKit');

function stats(path) {
  var rep = $.NSBitmapImageRep.imageRepWithContentsOfFile(path);
  if (!rep || rep.isNil()) throw new Error('not a picture: ' + path);
  var w = Number(rep.pixelsWide), h = Number(rep.pixelsHigh), seen = {}, clear = 0;
  for (var y = 0; y < 40; y++) {
    for (var x = 0; x < 64; x++) {
      var c = rep.colorAtXY(Math.floor(((x + 0.5) * w) / 64), Math.floor(((y + 0.5) * h) / 40));
      if (c.alphaComponent < 0.5) clear++;
      seen[[c.redComponent, c.greenComponent, c.blueComponent].map(function (v) { return Math.round(v * 255); }).join()] = 1;
    }
  }
  return JSON.stringify({ w: w, h: h, colors: Object.keys(seen).length, clear: clear });
}

// Pixels of a picture, as a bitmap of its own size (not its points).
function load(path) {
  var rep = $.NSBitmapImageRep.imageRepWithContentsOfFile(path);
  if (!rep || rep.isNil()) throw new Error('not a picture: ' + path);
  return rep;
}

function pair(before, after, out, leftLabel, rightLabel) {
  var reps = (before === '-' ? [] : [load(before)]).concat([load(after)]);
  var labels = before === '-' ? [rightLabel] : [leftLabel, rightLabel];
  var gap = 24, bar = 64;
  var width = gap, height = 0;
  reps.forEach(function (r) { width += Number(r.pixelsWide) + gap; height = Math.max(height, Number(r.pixelsHigh)); });
  height += bar + gap;
  var canvas = $.NSBitmapImageRep.alloc.initWithBitmapDataPlanesPixelsWidePixelsHighBitsPerSampleSamplesPerPixelHasAlphaIsPlanarColorSpaceNameBytesPerRowBitsPerPixel(
    null, width, height, 8, 4, true, false, $.NSDeviceRGBColorSpace, 0, 0);
  var context = $.NSGraphicsContext.graphicsContextWithBitmapImageRep(canvas);
  $.NSGraphicsContext.saveGraphicsState;
  $.NSGraphicsContext.setCurrentContext(context);
  $.NSColor.colorWithDeviceRedGreenBlueAlpha(0.93, 0.93, 0.93, 1).setFill;
  $.NSRectFill($.NSMakeRect(0, 0, width, height));
  var font = $.NSFont.boldSystemFontOfSize(36);
  var x = gap;
  reps.forEach(function (r, i) {
    // AppKit's origin is the bottom left: the label sits above the picture.
    var rw = Number(r.pixelsWide), rh = Number(r.pixelsHigh), y = height - bar - rh;
    var image = $.NSImage.alloc.initWithSize($.NSMakeSize(rw, rh));
    image.addRepresentation(r);
    image.drawInRectFromRectOperationFraction($.NSMakeRect(x, y, rw, rh), $.NSZeroRect, $.NSCompositingOperationSourceOver, 1);
    var attrs = $.NSDictionary.dictionaryWithObjectsForKeys(
      $([font, $.NSColor.blackColor]), $([$.NSFontAttributeName, $.NSForegroundColorAttributeName]));
    $(labels[i]).drawAtPointWithAttributes($.NSMakePoint(x, height - bar + 12), attrs);
    x += rw + gap;
  });
  context.flushGraphics;
  $.NSGraphicsContext.restoreGraphicsState;
  var png = canvas.representationUsingTypeProperties($.NSBitmapImageFileTypePNG, $());
  if (!png.writeToFileAtomically(out, true)) throw new Error('could not write ' + out);
  return out;
}

function run(argv) {
  if (argv[0] === 'stats') return stats(argv[1]);
  if (argv[0] === 'pair') return pair(argv[1], argv[2], argv[3], argv[4], argv[5]);
  throw new Error('usage: stats <png> | pair <before|-> <after> <out> <left> <right>');
}
