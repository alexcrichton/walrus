* Read in all `read::Dwarf`
* Convert to `write::Dwarf`
* Iterate units
  * Iterate entries
    * for all subprograms, update low-pc and high-pc
  * sort entries by address, then create a line program
