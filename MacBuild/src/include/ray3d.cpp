/*
 * ray3d.cpp

    Copyright � 2010 John Serafino
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

RayTyp::RayTyp()  {  }


dimension2d<u32> getScreenResolution(void)
{
    // create a NULL device to detect screen resolution
    irr::IrrlichtDevice *nulldevice = createDevice(video::EDT_NULL);
    core::dimension2d<u32> deskres = nulldevice->getVideoModeList()->getDesktopResolution();
    //nulldevice->drop();
    
    return deskres;
}

/* for initializing video */
void RayTyp::init(){
    
    dimension2d<u32> res = getScreenResolution();
    
    irr::SIrrlichtCreationParameters params;
    params.AntiAlias = 4;
    params.Bits = 32;
    params.DriverType = video::EDT_OPENGL;
    //params.EventReceiver = &rcv;
    params.Fullscreen = false;
    params.WindowSize = res;
    params.Stencilbuffer = false;
    
    std::cout << "Fullscreen: " << std::endl;
    std::cout << params.Fullscreen << std::endl;
    
    videoDevice = irr::createDeviceEx(params);
    //videoDevice = createDevice(EDT_OPENGL, res, 32, false, false, false);

    Video=videoDevice->getVideoDriver();
    Scene=videoDevice->getSceneManager();
    Gui=videoDevice->getGUIEnvironment();

    // default window title
    videoDevice->setWindowCaption(L"ray3d v0.01 experimental");

    // demand 32 bit textures
    Video->setTextureCreationFlag(ETCF_ALWAYS_32_BIT, true);

    // 2d image filtering
    Video->getMaterial2D().TextureLayer[0].BilinearFilter=true;
    Video->getMaterial2D().AntiAliasing=video::EAAM_FULL_BASIC;

    // set skinning mode
    useHwSkinning = true;
}

void RayTyp::setWindowTitle(wchar_t *title){
    videoDevice->setWindowCaption(title);
}

void RayTyp::hideCursor(){
    videoDevice->getCursorControl()->setVisible(false);
}
void RayTyp::showCursor(){
    videoDevice->getCursorControl()->setVisible(true);
}
void RayTyp::placeCursor(f32 x, f32 y){
    videoDevice->getCursorControl()->setPosition(x,y);
}

void RayTyp::importZipFile(char *filename){
    videoDevice->getFileSystem()->addZipFileArchive(filename);
}

// returns weather or not ray3d wants to be running
bool RayTyp::running(void){
    return videoDevice->run();
}

void RayTyp::exit(void){
    videoDevice->drop();
}

u32 RayTyp::getTime(){
    return videoDevice->getTimer()->getTime();
}

RayTyp Ray;
