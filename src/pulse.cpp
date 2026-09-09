/*
    Copyright ù 2011 John Serafino
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

#include "pulse.h"
#include "globals.h"

f32 getRand(f32 low, f32 high)
{
    return ((f32(rand()) / f32(RAND_MAX)) * (high - low)) + low;
}

PulseSet::~PulseSet()
{
    delete[] pulse;
    delete[] speed;
    pulse = nullptr;
    speed = nullptr;
}

void PulseSet::init(int number, int xSize, int ySize)
{
    worldX = xSize;
    worldY = ySize;

    pulse = new Entity[number];
    speed = new f32[number];

    pulseCount = number;

    int i;
    for(i=0; i < pulseCount; i++)
    {
        pulse[i].createMesh();
        pulse[i].loadMesh(gibson_config::pulse_mesh, false, false);
        pulse[i].loadTex(gibson_config::pulse_texture);
        pulse[i].setLit(false);
        pulse[i].setPosition(TOWER_DIST * int(getRand(-worldX, worldX)), getRand(1,MAX_PULSE_HEIGHT), \
                (TOWER_DIST * int(getRand(-worldY, worldY)) + TOWER_DIST/2));
        pulse[i].translateGlobal(getRand(-TOWER_DIST/4, TOWER_DIST/4),0,getRand(-TOWER_DIST/4, TOWER_DIST/4));

        pulse[i].sceneNode->setMaterialType(EMT_TRANSPARENT_ALPHA_CHANNEL);
        pulse[i].sceneNode->setMaterialFlag(EMF_TRILINEAR_FILTER, true);
        pulse[i].sceneNode->setMaterialFlag(EMF_ANISOTROPIC_FILTER, true);
        pulse[i].setScale(2,2,4);
        pulse[i].setRotation(0,90 * int(getRand(0,4)),0);

        speed[i] = getRand(MIN_PULSE_SPEED, MAX_PULSE_SPEED);

        if ((i % 32) == 0)
        {
            Ray.pumpEvents();
            if (Ray.quitRequested())
            {
                pulseCount = i + 1;
                break;
            }
        }
    }
}

void PulseSet::update()
{
    const f32 maxX = ((worldX/2) * TOWER_DIST);
    const f32 maxZ = ((worldY/2) * TOWER_DIST);

    int i;
    for(i=0; i < pulseCount; i++)
    {
        pulse[i].translate(0,0,-delta * speed[i]);

        if(pulse[i].getPosition().X >= maxX || pulse[i].getPosition().Z >= maxZ || \
                pulse[i].getPosition().X <= -maxX || pulse[i].getPosition().Z <= -maxZ)
        {

            pulse[i].setPosition(TOWER_DIST * int(getRand(-worldX, worldX)), getRand(1,MAX_PULSE_HEIGHT), \
                                        (TOWER_DIST * int(getRand(-worldY, worldY)) + TOWER_DIST/2));
            pulse[i].translateGlobal(getRand(-TOWER_DIST/4, TOWER_DIST/4),0,getRand(-TOWER_DIST/4, TOWER_DIST/4));

            pulse[i].setRotation(0,90 * int(getRand(0,4)),0);
            speed[i] = getRand(MIN_PULSE_SPEED, MAX_PULSE_SPEED);
        }

    }
}
