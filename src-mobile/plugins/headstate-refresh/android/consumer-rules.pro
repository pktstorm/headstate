# Tauri finds plugin classes and their @Command methods by reflection,
# and WorkManager instantiates the worker by class name; R8 must not
# rename or strip either.
-keep class com.pktstorm.headstate.refresh.** { *; }
