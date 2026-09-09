#ifndef GIBSON_SCENE_H_
#define GIBSON_SCENE_H_

struct GibsonSceneOptions
{
    unsigned width = 1280;
    unsigned height = 800;
    void *windowId = nullptr;
    void *bindView = nullptr;
    bool embedded = false;
    bool preview = false;
    bool fullscreen = false;
    const char *resourceDir = nullptr;
    const char *argv0 = nullptr;
};

struct GibsonFrame
{
    unsigned width = 0;
    unsigned height = 0;
    const unsigned char *bgra = nullptr;
};

bool gibson_scene_start(const GibsonSceneOptions &opts);
void gibson_scene_resize(unsigned width, unsigned height);
void gibson_scene_tick();
void gibson_scene_stop();
bool gibson_scene_ready();

/* Latest embedded frame (BGRA, top-down). Valid until the next tick/stop. */
bool gibson_scene_last_frame(GibsonFrame *frame);

#endif
