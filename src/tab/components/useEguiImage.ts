import {useEffect, useState, type RefObject} from 'react';
import {EguiImage, loadEgui} from '../egui';
import {renderPixels, type Frame} from '../frames';
import {report} from '../reporter';

// Stats are sent at most this often per image while it is drawing.
const REPORT_INTERVAL_MS = 500;
const MS_PER_SECOND = 1000;

// The effective transform is the wall's (tilt) composed with the card's
// (fan); devtools shows both so a reader can see why input still lines up.
const describeTransform = (card: Element) => {
    const parts = [card.closest('.wall'), card]
        .map(element => (element ? getComputedStyle(element).transform : 'none'))
        .filter(transform => transform !== 'none');
    return parts.length ? parts.join(' then ') : 'none';
};

export const useEguiImage = (canvasRef: RefObject<HTMLCanvasElement | null>, frame: Frame) => {
    const [failure, setFailure] = useState<string | null>(null);

    useEffect(() => {
        const canvas = canvasRef.current;
        if (!canvas) return;
        let image: EguiImage | null = null;
        let disposed = false;
        let animationFrame = 0;
        let timer = 0;
        let lastReport = 0;
        let frameMs = 0;
        let pointer: [number, number] | null = null;
        let cssSize = {width: 0, height: 0};
        let pixelSize = {width: 0, height: 0};

        const sendStats = (now: number) => {
            if (!image || now - lastReport < REPORT_INTERVAL_MS) return;
            lastReport = now;
            report({
                kind: 'image',
                id: frame.id,
                title: frame.title,
                cssWidth: cssSize.width,
                cssHeight: cssSize.height,
                pixelWidth: pixelSize.width,
                pixelHeight: pixelSize.height,
                zoom: image.zoom(),
                frameMs,
                pointer,
                transform: describeTransform(canvas.closest('.egui-card') ?? canvas),
            });
        };

        const draw = (now: number) => {
            animationFrame = 0;
            if (!image) return;
            const start = performance.now();
            const nextSeconds = image.frame(now / MS_PER_SECOND);
            frameMs = performance.now() - start;
            canvas.style.cursor = image.cursor();
            sendStats(now);
            schedule(nextSeconds);
        };

        const request = () => {
            if (!animationFrame) animationFrame = requestAnimationFrame(draw);
        };

        // egui says when it next wants a frame: now, later, or not until
        // more input arrives (infinity). Idle images cost nothing.
        const schedule = (seconds: number) => {
            window.clearTimeout(timer);
            if (!Number.isFinite(seconds)) return;
            if (seconds <= 0) request();
            else timer = window.setTimeout(request, seconds * MS_PER_SECOND);
        };

        // ResizeObserver reports layout size, which CSS transforms do not
        // change, so the backing store tracks the slider, not the tilt.
        const applySize = (entry: ResizeObserverEntry) => {
            const box = entry.contentBoxSize[0];
            const device = entry.devicePixelContentBoxSize?.[0];
            cssSize = {width: box.inlineSize, height: box.blockSize};
            pixelSize = {
                width: Math.max(1, Math.round(device?.inlineSize ?? box.inlineSize * window.devicePixelRatio)),
                height: Math.max(1, Math.round(device?.blockSize ?? box.blockSize * window.devicePixelRatio)),
            };
            canvas.width = pixelSize.width;
            canvas.height = pixelSize.height;
            image?.resize(cssSize.width, cssSize.height, pixelSize.width, pixelSize.height);
            request();
        };
        const observer = new ResizeObserver(entries => entries.forEach(applySize));
        try {
            observer.observe(canvas, {box: 'device-pixel-content-box'});
        } catch {
            // WebKit lacks device-pixel-content-box; devicePixelRatio is the fallback.
            observer.observe(canvas);
        }

        // offsetX/offsetY are in the target's own coordinate space with every
        // ancestor transform undone, which is exactly egui's screen space.
        const onMove = (event: PointerEvent) => {
            pointer = [event.offsetX, event.offsetY];
            image?.pointer_moved(event.offsetX, event.offsetY);
            request();
        };
        const onDown = (event: PointerEvent) => {
            canvas.setPointerCapture(event.pointerId);
            image?.pointer_button(event.offsetX, event.offsetY, event.button, true);
            request();
        };
        const onUp = (event: PointerEvent) => {
            image?.pointer_button(event.offsetX, event.offsetY, event.button, false);
            request();
        };
        // While captured mid-drag the pointer counts as over the canvas, so
        // this only fires once the drag (if any) is over.
        const onLeave = () => {
            pointer = null;
            image?.pointer_left();
            request();
            lastReport = 0;
        };
        // Pinch arrives as ctrl+wheel; plain wheel keeps scrolling the page.
        const onWheel = (event: WheelEvent) => {
            if (!event.ctrlKey || !image) return;
            event.preventDefault();
            image.pinch(event.deltaX, event.deltaY, event.deltaMode);
            request();
        };
        canvas.addEventListener('pointermove', onMove);
        canvas.addEventListener('pointerdown', onDown);
        canvas.addEventListener('pointerup', onUp);
        canvas.addEventListener('pointerleave', onLeave);
        canvas.addEventListener('wheel', onWheel, {passive: false});

        loadEgui()
            .then(() => {
                if (disposed) return;
                const pixels = renderPixels(frame);
                image = new EguiImage(canvas, frame.title, pixels.width, pixels.height, pixels.rgba);
                image.resize(cssSize.width, cssSize.height, pixelSize.width, pixelSize.height);
                request();
            })
            .catch((error: unknown) => {
                const message = error instanceof Error ? error.message : String(error);
                console.error(`${frame.title}: egui could not start: ${message}`);
                if (!disposed) setFailure(message);
            });

        return () => {
            disposed = true;
            cancelAnimationFrame(animationFrame);
            window.clearTimeout(timer);
            observer.disconnect();
            canvas.removeEventListener('pointermove', onMove);
            canvas.removeEventListener('pointerdown', onDown);
            canvas.removeEventListener('pointerup', onUp);
            canvas.removeEventListener('pointerleave', onLeave);
            canvas.removeEventListener('wheel', onWheel);
            if (image) {
                image.destroy();
                image.free();
            }
        };
    }, [canvasRef, frame]);

    return failure;
};
