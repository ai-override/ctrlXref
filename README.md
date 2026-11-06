# From AI (Comet Assistant)

Question:
in Star Citizen which configuration file(s) hold the actual referenced value for mappings like  "UILabel"="@ui_COFlightPitch" seen inside the defaultProflie.xml


Answer : 
The actual display text is in the game’s localization data, primarily the packed global.ini localization database—not in defaultProfile.xml. @ui_COFlightPitch is a localization key: the @ prefix tells CryEngine/Star Citizen to look up that key and substitute the language-specific string.

Where to look
For the installed game, the relevant files are under:

text
<StarCitizen install>\LIVE\data\Localization\
Historically/community tooling exposes this as an English base global.ini (or localization archives that unpack to one), with keys such as:

text
ui_COFlightPitch = Flight Pitch
So UILabel="@ui_COFlightPitch" resolves to whatever value is assigned to ui_COFlightPitch for the currently selected language.

Important detail
Star Citizen normally loads one localization file. If a key is absent, the UI can display the raw token, e.g. @ui_COFlightPitch, rather than the intended label.

English strings are the base/reference values.

Other languages are layered over that English base, with English used as fallback for untranslated entries.

Community localization projects maintain extracted/translated versions of global.ini, which is why that filename is the practical target for searching.

Practical search
If you want to inspect the exact current value, search the extracted localization files for:

text
ui_COFlightPitch
or, in the raw packed game data, search for the key without the @:

text
ui_COFlightPitch
Do not search for @ui_COFlightPitch in the localization file itself; the @ is a lookup prefix used by the XML/profile layer, not part of the stored key.

This question looks difficult
Try a more powerful model to trace where those UI string keys resolve and how the input mapping pipeline loads them.

Architect Reviews the Constellation Mk5 - Star Citizen - YouTube




# D: 
cd D:\Devel\rust_projects\ctrlXref
claude 
Shift + tab until correct mode ( plan )
 

#  Execution
cmd 
d: 
cd D:\Devel\rust_projects\ctrlXref\target\debug

D:\Devel\rust_projects\ctrlXref\target\debug>ctrlxref.exe

Opens the extracted Data\Libs\Config\defaultProfile.xml of the located Star Citizen folder.
An optional first argument is a profile path relative to that folder (or absolute), the
optional second one a rebind file relative to the current directory:

ctrlxref.exe Data\Libs\Config\defaultProfile.xml ..\..\xml\actionmaps.xml

If the Data folder is missing, the app offers to extract Data.p4k first.
