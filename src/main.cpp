/* Gibson - Screensaver that pays homage to the Gibson in Hackers */

/*
    Copyright ? 2011 John Serafino
    This file is part of The Gibson Screensaver.

    The Gibson Screensaver is free software: you can redistribute it and/or modify
    it under the terms of the GNU Lesser General Public License as published by
    the Free Software Foundation, either version 3 of the License, or
    (at your option) any later version.

    The Gibson Screensaver is distributed in the hope that it will be useful,
    but WITHOUT ANY WARRANTY; without even the implied warranty of
    MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
    GNU Lesser General Public License for more details.

    You should have received a copy of the GNU Lesser General Public License
    along with The Gibson Screensaver If not, see <http://www.gnu.org/licenses/>.
*/

#include "events.h"
#include "gibson_scene.h"
#include "macos_retina.h"
#include "ray3d.h"

#include <cstring>
#include <iostream>
#include <stdexcept>

static const wchar_t *gibson_version = L"The Gibson version 15";

static void print_usage(const char *argv0)
{
    std::cout << "Usage: " << argv0 << " [--fullscreen] [--help]\n"
              << "  --fullscreen   Borderless full-display flythrough\n"
              << "  --help         Show this message\n"
              << "Press Escape or Q to quit.\n";
}

int main(int argc, char **argv)
{
    bool fullscreen = false;
    for (int i = 1; i < argc; ++i)
    {
        if (std::strcmp(argv[i], "--fullscreen") == 0)
            fullscreen = true;
        else if (std::strcmp(argv[i], "--help") == 0 || std::strcmp(argv[i], "-h") == 0)
        {
            print_usage(argv[0]);
            return 0;
        }
        else
        {
            std::cerr << "Unknown argument: " << argv[i] << "\n";
            print_usage(argv[0]);
            return 1;
        }
    }

    unsigned desk_w = 0, desk_h = 0;
    gibson_get_desktop_size(&desk_w, &desk_h);

    GibsonSceneOptions opts;
    opts.argv0 = argv[0];
    opts.fullscreen = fullscreen;
    if (fullscreen)
    {
        opts.width = desk_w;
        opts.height = desk_h;
    }
    else
    {
        const unsigned w = desk_w > 1280 ? 1280 : (desk_w ? desk_w : 1280);
        const unsigned h = desk_h > 800 ? 800 : (desk_h ? desk_h : 800);
        opts.width = w;
        opts.height = h;
    }

    try
    {
        if (!gibson_scene_start(opts))
            return 1;
    }
    catch (const std::exception &ex)
    {
        std::cerr << ex.what() << "\n";
        gibson_scene_stop();
        return 1;
    }

    int lastFPS = Video ? Video->getFPS() : 0;
    int fps;

    while (Ray.running() && !key[KEY_ESCAPE] && !key[KEY_KEY_Q] && !gibson_should_quit())
    {
        if (key[KEY_F10])
            Ray.Render.takeScreenshot();

        gibson_scene_tick();

        if (!Video || !irrlicht)
            break;

        fps = Video->getFPS();
        if (lastFPS != fps)
        {
            core::stringw str = gibson_version;
            str += " FPS: ";
            str += fps;
            irrlicht->setWindowCaption(str.c_str());
            lastFPS = fps;
        }
    }

    std::cout << "Made it to end! Attempting Ray.exit..." << std::endl;
    gibson_scene_stop();
    std::cout << "Ray3D is dead!" << std::endl;
    return 0;
}
