/* Gibson - Screensaver that pays homage to the Gibson in Hackers */

/*
    Copyright © 2011 John Serafino
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

//-Wl,-subsystem,windows

#include "main.h"
#include "pathCreator.h"

// Global variables
f32 then, delta;

wchar_t* gibson_version = L"The Gibson version 15";
char* loading_image = "media/loading.png";


int main(int argc, char **argv){

    //Ray.init(OGL, 800,600, false, false, false);

    dimension2d<u32> res = getScreenResolution();

    SIrrlichtCreationParameters params;
    params.AntiAlias = 4;
    params.Bits = 32;
    params.DriverType = EDT_OPENGL;
    params.EventReceiver = &rcv;
    params.Fullscreen = true;
    params.WindowSize = res;
    params.Stencilbuffer = false;
    params.WindowSize = res;

    Ray.init(params);

    /* Window created */

    //irrlicht->getFileSystem()->changeWorkingDirectoryTo("C:\\Gibson\\");

    Ray.hideCursor();
    Video->setAllowZWriteOnTransparent(true);

    IPostProc* ppRenderer = new CRendererPostProc(
        Scene,
        dimension2du(1024, 1024),
        true,
        true,
        SColor(255u, 0, 10, 15)
    );

    // When setting up the effect, the parameters are:
    // Input, size of output, effect ID (see CEffectPostProc.h for full
    // list), effect parameters (in this case, blur size)
    /*
    CEffectPostProc* ppBlur = new CEffectPostProc(
        ppRenderer,
        dimension2du(1024, 1024),
        PP_BLOOM,
        3,
        0.005,
        1.5
    );
    CEffectPostProc* ppBlur = new CEffectPostProc(
        ppRenderer,
        dimension2du(1024, 1024),
        PP_INVERT
    );
    */
    CEffectPostProc* ppBlur = new CEffectPostProc(
        ppRenderer,
        dimension2du(1024, 1024),
        PP_MOTIONBLUR,
        0.1
    );

    // Change to a better quality - not all shaders will respect these,
    // but they can be used to hint the rendering standard required.
    // Options (worst to best): PPQ_CRUDE, PPQ_FAST, PPQ_DEFAULT,
    //     PPQ_GOOD, PPQ_BEST
    // You can also call setOverallQuality( PPQ_WHATEVER ) to change the
    // quality of all effects which are in the chain.
    ppBlur->setQuality( PPQ_BEST );

    //Ray.importZipFile("D:\\gibson\\data.zip");

    // Set window title
    Ray.setWindowTitle(gibson_version);

    // Draw loading image
    Image loading;
    loading.loadImg(loading_image);
    Ray.Render.clearScreen(0,0,0,1);
    loading.draw((res.Width/2) - (loading.w/2), (res.Height/2) - (loading.h/2));

    Gui->drawAll();
    Video->endScene();

    Room room;
    FlyCam cam;
    PulseSet pulses;
    srand(time(NULL));

    room.init();
    //cam.init();
    pulses.init(1000);

    setAmbient(0.4,0.4,0.4, 1);

    cam.init();
    cam.cam.camera->setFarValue(700);

    int lastFPS = Video->getFPS();
    then = Ray.getTime();

    bool kill = false;
    int fps;

    //PathCreator creator(Video, cam.cam.camera, "path.txt", "camPath");

    while(Ray.running() && !key[KEY_ESCAPE])
    {
        delta = (f32)(Ray.getTime() - then) / 1000;
        then = Ray.getTime();

        Ray.Render.clearScreen(0,10,15,1);

        ///*

        if(key[KEY_F10])
            Ray.Render.takeScreenshot();


        cam.empty->updateAbsolutePosition();
        cam.cam.camera->setTarget(cam.empty->getAbsolutePosition());
        room.update();
        pulses.update();

        /*
        for(int x=0; x < KEY_KEY_CODES_COUNT; x++)
        {
            if(x != KEY_F10 && key[x])
            {
                kill = true;
                break;
            }
        }
        if(kill)
            break;
            */


        /*
        if(leftMouse)
            creator.addPath(); // add path to list
        else if (rightMouse)
            creator.delPath(); // delete path from list
        else if (key[KEY_KEY_C])
            creator.conPath(); // connect path
        else if (key[KEY_KEY_Z])
            creator.save(); // save list to file


           creator.drawPath();
        Ray.Render.drawScreen();
        Ray.Render.endScene();
        */




       // The rendering is as normal, except smgr->drawAll(); is
       // replaced with this line:
       // NULL = render to screen. Can also take a texture to render to,
       // or no parameter (renders to an internal texture)
       ppBlur->render( NULL );
       //*/
       //Scene->drawAll();

       Gui->drawAll();
       Video->endScene();


       // Change the window title reflect changes in the FPS count
       fps = Video->getFPS();
        if (lastFPS != fps){
            core::stringw str = gibson_version;
            str += " FPS: ";
            str += fps;

            irrlicht->setWindowCaption(str.c_str());
            lastFPS = fps;
        }
    }

    delete ppBlur;
    delete ppRenderer;


    //cout << cam.cam.camera->getAbsolutePosition().X << ',' \
            //<< cam.cam.camera->getAbsolutePosition().Y << ',' \
            //<< cam.cam.camera->getAbsolutePosition().Z << '\n';
    //cout << cam.empty->getAbsolutePosition().X << ',' \
            //<< cam.empty->getAbsolutePosition().Y << ',' \
            //<< cam.empty->getAbsolutePosition().Z << '\n';

    cout << "Made it to end! Attempting Ray.exit..." << endl;

    Ray.exit();
    cout << "Ray3D is dead!" << endl;

    exit(0);

    // This shouldn't be necessary
    return 0;
}
