/* Gibson program global variables, mainly the location of files*/

/*
    Copyright © 2015 Paul Kiernan
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

#ifndef GLOBALS_H_
#define GLOBALS_H_

/* Configuration container for things like texture paths and mesh files
 * and etc.
 */
namespace gibson_config{

    char* const room_mesh = "media/room.3ds";
    char* const room_texture = "media/room.png";
    char* const towers_mesh = "media/towers.obj";
    char* const dark_towers_template_filename = "media/towers1-%d.png";
    char* const light_towers_template_filename = "media/towers2-%d.png";
    char* const pulse_mesh = "media/pulse.obj";
    char* const pulse_texture = "media/pulse.png";
};

#endif /* GLOBALS_H_ */
