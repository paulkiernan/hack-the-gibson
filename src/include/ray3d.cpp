/*
 * ray3d.cpp

    Copyright ? 2010 John Serafino
    This file is part of ray3d.

    Ray3d is free software: you can redistribute it and/or modify
    it under the terms of the GNU Lesser General Public License as published by
    the Free Software Foundation, either version 3 of the License, or
    (at your option) any later version.

    Ray3d v0.01 is distributed in the hope that it will be useful,
    but WITHOUT ANY WARRANTY; without even the implied warranty of
    MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
    GNU Lesser General Public License for more details.

    You should have received a copy of the GNU Lesser General Public License
    along with ray3d If not, see <http://www.gnu.org/licenses/>.
 */

#include "ray3d.h"
#include "macos_retina.h"

RayTyp::RayTyp()  {  }

/* for initializing video */
void RayTyp::init(SIrrlichtCreationParameters params){
    gibson_prepare_retina_workaround();
    irrlicht = createDeviceEx(params);
    if (!irrlicht)
        gibson_fatal("Failed to create an OpenGL window");

    Video=irrlicht->getVideoDriver();
    Scene=irrlicht->getSceneManager();
    Gui=irrlicht->getGUIEnvironment();

    // default window title
    irrlicht->setWindowCaption(L"ray3d v0.01 experimental");

    // demand 32 bit textures
    Video->setTextureCreationFlag(ETCF_ALWAYS_32_BIT, true);

    // 2d image filtering
    Video->getMaterial2D().TextureLayer[0].BilinearFilter=true;
    Video->getMaterial2D().AntiAliasing=video::EAAM_FULL_BASIC;

    // set skinning mode
    useHwSkinning = true;

    if (Video)
    {
        const core::dimension2d<u32>& sz = Video->getScreenSize();
        gibson_fix_retina_framebuffer(sz.Width, sz.Height);
        Video->setViewPort(rect<s32>(0, 0, (s32)sz.Width, (s32)sz.Height));
    }
}

void RayTyp::setWindowTitle(const wchar_t *title){
    irrlicht->setWindowCaption(title);
}

void RayTyp::hideCursor(){
    irrlicht->getCursorControl()->setVisible(false);
}
void RayTyp::showCursor(){
    irrlicht->getCursorControl()->setVisible(true);
}
void RayTyp::placeCursor(f32 x, f32 y){
    irrlicht->getCursorControl()->setPosition(x,y);
}

void RayTyp::importZipFile(const char *filename){
    irrlicht->getFileSystem()->addZipFileArchive(filename);
}

// returns weather or not ray3d wants to be running
bool RayTyp::running(void){
    if (gibson_should_quit() && irrlicht)
        irrlicht->closeDevice();

    if (Video)
    {
        const core::dimension2d<u32>& sz = Video->getScreenSize();
        gibson_sync_gl_backing(sz.Width, sz.Height);
    }
    if (gibson_is_embedded())
    {
        if (irrlicht)
            irrlicht->getTimer()->tick();
        return irrlicht != nullptr;
    }
    return irrlicht->run();
}

void RayTyp::pumpEvents(void){
    if (gibson_is_embedded())
        return;
    if (irrlicht)
        irrlicht->run();
}

bool RayTyp::quitRequested(void){
    return gibson_should_quit() != 0;
}

void RayTyp::exit(void){
    gibson_restore_presentation();
    if (gibson_is_embedded())
    {
        /* Irrlicht 1.8's macOS destructor crashes in COpenGLSLMaterialRenderer. */
        irrlicht = nullptr;
        Video = nullptr;
        Scene = nullptr;
        Gui = nullptr;
        return;
    }
    // Something weird is going on with reference counts
    // TODO: figure out what's wrong with calling drop()
    //irrlicht->drop();
}

u32 RayTyp::getTime(){
    return irrlicht->getTimer()->getTime();
}

RayTyp Ray;
