# Tauri finds plugin classes and their @Command methods by reflection;
# R8 must not rename or strip them.
-keep class com.pktstorm.headstate.keys.** { *; }
