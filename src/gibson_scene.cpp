#include "gibson_scene.h"

#include "cam.h"
#include "events.h"
#include "globals.h"
#include "macos_retina.h"
#include "pulse.h"
#include "ray3d.h"
#include "room.h"

#include <Effects/CEffectPostProc.h>
#include <Effects/CRendererPostProc.h>

#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <ctime>
#include <iostream>
#include <libgen.h>
#include <limits.h>
#include <sys/stat.h>
#include <string>
#include <vector>
#include <unistd.h>

f32 then = 0;
f32 delta = 0;

static const wchar_t *gibson_version = L"The Gibson version 15";
static const char *loading_image = "media/loading.png";

static bool g_ready = false;
static bool g_starting = false;
static bool g_embedded = false;
static IPostProc *g_ppRenderer = nullptr;
static CEffectPostProc *g_ppBlur = nullptr;
static Room *g_room = nullptr;
static FlyCam *g_cam = nullptr;
static PulseSet *g_pulses = nullptr;
static std::vector<unsigned char> g_frame_bgra;
static unsigned g_frame_w = 0;
static unsigned g_frame_h = 0;

static bool is_dir(const char *path)
{
    struct stat st;
    return stat(path, &st) == 0 && S_ISDIR(st.st_mode);
}

static void locate_assets(const char *argv0, const char *resourceDir)
{
    if (resourceDir && resourceDir[0])
    {
        if (chdir(resourceDir) != 0 || !is_dir("media"))
            gibson_fatal("Could not find media/ in the screensaver bundle");
        return;
    }

    if (is_dir("media"))
        return;

    if (is_dir("../media") && chdir("..") == 0)
        return;

    char exe[PATH_MAX];
    std::snprintf(exe, sizeof(exe), "%s", argv0 ? argv0 : "");
    char *dir = dirname(exe);
    if (dir && chdir(dir) == 0)
    {
        if (is_dir("media"))
            return;
        if (is_dir("../media") && chdir("..") == 0)
            return;
    }

    gibson_fatal("Could not find the media/ directory. Run from the project root or from src/");
}

static void bind_if_needed(void *bindView, unsigned width, unsigned height)
{
    if (!bindView)
        return;
    gibson_bind_gl_to_view(bindView, width, height);
}

bool gibson_scene_ready()
{
    return g_ready;
}

static void release_scene_objects()
{
    delete g_pulses;
    g_pulses = nullptr;
    delete g_cam;
    g_cam = nullptr;
    delete g_room;
    g_room = nullptr;
    delete g_ppBlur;
    g_ppBlur = nullptr;
    delete g_ppRenderer;
    g_ppRenderer = nullptr;
    g_frame_bgra.clear();
    g_frame_w = 0;
    g_frame_h = 0;
}

bool gibson_scene_start(const GibsonSceneOptions &opts)
{
    /* System Settings recreates ScreenSaverView constantly. Re-entering init
       mid-Ray.init() (nested run loop) tears the scene down and flashes. */
    if (opts.embedded)
    {
        if (g_ready && irrlicht && Video && Scene)
            return true;
        if (g_starting)
            return false;
        g_starting = true;
    }
    else
    {
        g_ready = false;
        release_scene_objects();
        if (irrlicht)
            Ray.exit();
    }

    struct StartingGuard
    {
        bool embedded;
        bool committed;
        explicit StartingGuard(bool e) : embedded(e), committed(false) {}
        ~StartingGuard()
        {
            if (embedded && !committed)
                g_starting = false;
        }
    } starting_guard(opts.embedded);

    g_embedded = opts.embedded;
    gibson_set_embedded(opts.embedded);

    std::string resourceDir = opts.resourceDir ? opts.resourceDir : "";
    locate_assets(opts.argv0, resourceDir.empty() ? nullptr : resourceDir.c_str());

    unsigned width = opts.width;
    unsigned height = opts.height;
    if (opts.embedded)
    {
        /* Blit to the saver view; Irrlicht's own window stays small. */
        width = opts.preview ? 512u : 1280u;
        height = opts.preview ? 512u : 720u;
    }
    else if (!width || !height)
    {
        gibson_get_desktop_size(&width, &height);
        if (!width || !height)
        {
            width = 1280;
            height = 800;
        }
    }

    SIrrlichtCreationParameters params;
    params.AntiAlias = opts.embedded ? 0 : 4;
    params.Bits = 32;
    params.DriverType = EDT_OPENGL;
    params.EventReceiver = opts.embedded ? nullptr : &rcv;
    params.Stencilbuffer = false;
    params.Fullscreen = false;
    params.IgnoreInput = opts.embedded;
    params.WindowSize = dimension2d<u32>(width, height);
    params.WindowId = opts.windowId;

    if (!(opts.embedded && irrlicht && Video && Scene))
    {
        if (opts.embedded)
            gibson_snapshot_windows();
        Ray.init(params);
        if (opts.embedded)
            gibson_cloak_new_windows();
    }
    if (!irrlicht || !Video)
        gibson_fatal("Failed to create an OpenGL window. Irrlicht could not initialize the video device.");

    if (!resourceDir.empty())
    {
        if (chdir(resourceDir.c_str()) != 0 || !is_dir("media"))
            gibson_fatal("Could not find media/ in the screensaver bundle");
        irrlicht->getFileSystem()->changeWorkingDirectoryTo(resourceDir.c_str());
    }

    if (!opts.embedded)
        std::atexit(gibson_restore_presentation);

    if (opts.fullscreen && !opts.embedded)
    {
        gibson_enter_borderless_fullscreen();
        const core::dimension2d<u32> sz = Video->getScreenSize();
        gibson_fix_retina_framebuffer(sz.Width, sz.Height);
        Video->OnResize(params.WindowSize);
        Video->setViewPort(rect<s32>(0, 0, (s32)params.WindowSize.Width,
                                     (s32)params.WindowSize.Height));
    }

    bind_if_needed(opts.bindView, width, height);

    if (!opts.embedded)
    {
        gibson_install_quit_monitor();
        clearKeys();
        Ray.hideCursor();
        Ray.setWindowTitle(gibson_version);
    }

    Video->setAllowZWriteOnTransparent(true);

    const u32 pp = opts.preview ? 512u : (opts.embedded ? 1280u : 1024u);
    g_ppRenderer = new CRendererPostProc(
        Scene,
        dimension2du(pp, pp),
        true,
        true,
        SColor(255u, 0, 10, 15)
    );
    g_ppBlur = new CEffectPostProc(
        g_ppRenderer,
        dimension2du(pp, pp),
        PP_MOTIONBLUR,
        0.1
    );
    g_ppBlur->setQuality(opts.preview ? PPQ_FAST : PPQ_BEST);

    if (!opts.embedded)
    {
        Image loading;
        loading.loadImg(loading_image);
        Ray.Render.clearScreen(0, 0, 0, 1);
        const dimension2d<u32> win = Video->getScreenSize();
        loading.draw((win.Width / 2) - (loading.w / 2), (win.Height / 2) - (loading.h / 2));
        Gui->drawAll();
        Video->endScene();
    }

    std::srand((unsigned)std::time(nullptr));

    g_room = new Room();
    g_cam = new FlyCam();
    g_pulses = new PulseSet();

    g_room->init(opts.preview);
    if (Ray.quitRequested())
    {
        g_starting = false;
        gibson_scene_stop();
        return false;
    }

    const int pulseCount = opts.preview ? 40 : 1000;
    g_pulses->init(pulseCount, g_room->gridX, g_room->gridY);
    if (Ray.quitRequested())
    {
        g_starting = false;
        gibson_scene_stop();
        return false;
    }

    setAmbient(0.4, 0.4, 0.4, 1);
    g_cam->init();
    g_cam->cam.camera->setFarValue(700);

    then = Ray.getTime();
    g_ready = true;
    starting_guard.committed = true;
    g_starting = false;
    return true;
}

void gibson_scene_resize(unsigned width, unsigned height)
{
    if (!g_ready || !Video || !width || !height)
        return;

    Video->OnResize(dimension2d<u32>(width, height));
    Video->setViewPort(rect<s32>(0, 0, (s32)width, (s32)height));
    gibson_sync_gl_backing(width, height);
}

static void capture_embedded_frame()
{
    if (!g_ppBlur || !Video)
        return;

    ITexture *tex = g_ppBlur->getOutputTexture();
    if (!tex)
        return;

    const dimension2d<u32> sz = tex->getSize();
    IImage *img = Video->createImage(tex, position2d<s32>(0, 0), sz);
    if (!img)
        return;

    const u32 w = img->getDimension().Width;
    const u32 h = img->getDimension().Height;
    const u32 pitch = img->getPitch();
    const u8 *src = static_cast<const u8 *>(img->lock());
    if (src && w && h)
    {
        std::vector<unsigned char> next(static_cast<size_t>(w) * h * 4u);
        for (u32 y = 0; y < h; ++y)
        {
            unsigned char *dst = &next[(h - 1 - y) * w * 4u];
            std::memcpy(dst, src + y * pitch, static_cast<size_t>(w) * 4u);
            for (u32 x = 0; x < w; ++x)
                dst[x * 4u + 3u] = 255;
        }
        g_frame_bgra.swap(next);
        g_frame_w = w;
        g_frame_h = h;
    }
    img->unlock();
    img->drop();
}

void gibson_scene_tick()
{
    if (!g_ready || !Video || !irrlicht)
        return;

    if (g_embedded)
        irrlicht->getTimer()->tick();

    delta = (f32)(Ray.getTime() - then) / 1000;
    if (delta < 0)
        delta = 0;
    if (delta > 0.1f)
        delta = 0.1f;
    then = Ray.getTime();

    if (g_cam && g_cam->empty && g_cam->cam.camera)
    {
        g_cam->empty->updateAbsolutePosition();
        g_cam->cam.camera->setTarget(g_cam->empty->getAbsolutePosition());
    }
    if (g_room)
        g_room->update();
    if (g_pulses)
        g_pulses->update();

    if (g_embedded)
    {
        if (g_ppBlur)
            g_ppBlur->render();
        capture_embedded_frame();
        return;
    }

    Ray.Render.clearScreen(0, 10, 15, 1);
    if (g_ppBlur)
        g_ppBlur->render(NULL);
    if (Gui)
        Gui->drawAll();
    Video->endScene();
}

bool gibson_scene_last_frame(GibsonFrame *frame)
{
    if (!frame || g_frame_bgra.empty() || !g_frame_w || !g_frame_h)
        return false;
    frame->width = g_frame_w;
    frame->height = g_frame_h;
    frame->bgra = g_frame_bgra.data();
    return true;
}

void gibson_scene_stop()
{
    g_ready = false;
    g_starting = false;
    release_scene_objects();

    if (g_embedded)
    {
        /* Keep the Irrlicht device; dropping it crashes Irrlicht 1.8 on macOS. */
        gibson_set_embedded(false);
        g_embedded = false;
        return;
    }

    if (irrlicht)
        Ray.exit();

    gibson_set_embedded(false);
    g_embedded = false;
}
